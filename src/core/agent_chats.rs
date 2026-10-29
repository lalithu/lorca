use super::agent::CoreInference;
use super::*;
use crate::agent::{Limits, Registry, Status, Submission, Workspace};

impl Core {
    pub(super) fn agent_in_chat(&self, id: &str) -> bool {
        self.agent
            .snapshot()
            .is_some_and(|t| t.status.active() && t.conversation_id.as_deref() == Some(id))
    }
    pub(super) fn cancel_chat_navigation(&self, s: &mut State, session: &str) {
        if !self
            .agent
            .snapshot()
            .is_some_and(|t| t.status.active() && t.owner == session)
        {
            Self::cancel_session(s, session);
            return;
        }
        // Switching chats must not cancel the agent's model preparation.
        for turn in s
            .view
            .turns
            .iter()
            .filter(|t| t.session_id == session && t.running)
        {
            if let Some(tx) = s.cancels.get(&turn.id) {
                tx.send_replace(true);
            }
        }
    }
    pub(super) fn start_agent(self: &Arc<Self>, session: &str, v: &Value) -> Result<Value> {
        let mut s = self.state.lock().unwrap();
        let si = Self::session_index(&s, session)?;
        let model = v["model"]
            .as_str()
            .unwrap_or(&s.view.sessions[si].model)
            .to_string();
        if model.is_empty() || model.len() > 500 || model.chars().any(char::is_control) {
            return Err("Invalid model name".into());
        }
        let limits = if v.get("limits").is_some() {
            serde_json::from_value::<Limits>(v["limits"].clone()).map_err(|e| e.to_string())?
        } else {
            Limits::default()
        };
        limits.validate()?;
        let task = v["task"].as_str().unwrap_or("").trim();
        if (task.is_empty() && v["conversationId"].is_null()) || task.len() > 8000 {
            return Err("Task must contain 1–8000 bytes".into());
        }
        let workspace = v["workspace"]
            .as_str()
            .filter(|p| !p.is_empty())
            .map(|p| Workspace::new(std::path::Path::new(p)))
            .transpose()?;
        let ci = if let Some(id) = v["conversationId"].as_str() {
            Some(Self::chat_index(&s, id)?)
        } else {
            None
        };
        let request = if ci.is_some() {
            Some(crate::storage::valid_id(text(v, "requestId")?).map_err(|e| e.to_string())?)
        } else {
            None
        };
        if let Some(ci) = ci {
            if let Some(message) = s.view.conversations[ci].messages.iter().find(|m| {
                m.extra.get("agentRequestId").and_then(Value::as_str) == request.as_deref()
            }) {
                return Ok(json!({"taskId":message.extra["agentTaskId"],"alreadyAccepted":true}));
            }
            if s.view
                .turns
                .iter()
                .any(|t| t.running && t.conversation_id == s.view.conversations[ci].id)
            {
                return Err("Stop the current reply before starting an agent task.".into());
            }
        }
        if let Some(previous) = self.agent.snapshot() {
            if previous.status.active() {
                return Err("An agent task is already active".into());
            }
            if let Some(id) = previous.conversation_id
                && s.view.conversations.iter().any(|c| c.id == id)
                && !s.view.conversations.iter().any(|c| {
                    c.id == id
                        && c.messages.iter().any(|m| {
                            m.role == "assistant"
                                && m.extra.get("agentTaskId").and_then(Value::as_str)
                                    == Some(&previous.id)
                        })
                })
            {
                return Err("The previous agent result could not be saved. Reconnect before starting another task.".into());
            }
        }
        let effective_task = if task.is_empty() {
            "Describe the attached images.".to_string()
        } else {
            task.to_string()
        };
        let mut resume_evidence = vec![];
        let mut parent = None;
        let mut pending_task = None;
        if let Some(ci) = ci {
            let chat = &s.view.conversations[ci];
            if let Some(previous) = chat.messages.last().filter(|m| {
                m.role == "assistant"
                    && (m.extra.get("agentStatus") == Some(&json!("waiting_for_input"))
                        || m.extra.get("agentContract").is_some_and(|c| {
                            c["relation"] == "status" && c["pendingTask"].is_object()
                        })
                        || (v["continueTaskId"].as_str().is_some()
                            && m.extra.get("agentTaskId") == v.get("continueTaskId")
                            && matches!(
                                m.extra.get("agentStatus").and_then(Value::as_str),
                                Some("incomplete" | "failed" | "cancelled")
                            )))
                    && m.extra
                        .get("agentWorkspace")
                        .and_then(Value::as_str)
                        .or_else(|| {
                            request_for(chat, m)
                                .and_then(|request| request.extra.get("agentWorkspace"))
                                .and_then(Value::as_str)
                        })
                        .unwrap_or("")
                        == workspace
                            .as_ref()
                            .map(|w| w.root.to_str().unwrap_or(""))
                            .unwrap_or("")
            }) {
                let original = previous
                    .extra
                    .get("agentObjective")
                    .and_then(Value::as_str)
                    .or_else(|| request_for(chat, previous).map(|request| request.content.as_str()))
                    .ok_or("Waiting task has no saved objective")?;
                pending_task = Some(crate::agentloop::intent::PendingTask {
                    completed: false,
                    id: previous.extra["agentTaskId"].as_str().unwrap_or("").into(),
                    objective: original.into(),
                    contract: previous.extra.get("agentContract").cloned(),
                });
                if let Some(saved) = previous
                    .extra
                    .get("agentContract")
                    .filter(|c| c["relation"] == "status")
                    .and_then(|c| c.get("pendingTask"))
                    && let Ok(task) = serde_json::from_value::<crate::agentloop::intent::PendingTask>(
                        saved.clone(),
                    )
                {
                    pending_task = Some(task);
                }
                if effective_task.len() > 8000 {
                    return Err("The continued objective is too long; start a new request with a concise objective.".into());
                }
                parent = previous.extra.get("agentTaskId").cloned();
                resume_evidence =
                    linked_evidence(chat, parent.as_ref().and_then(Value::as_str).unwrap_or(""))?;
            }
        }
        // Completed tasks inform short follow-ups, but are not resumable work.
        // In particular, do not link their old execution evidence to a new turn.
        if pending_task.is_none()
            && v["continueTaskId"].is_null()
            && let Some(ci) = ci
            && let Some(previous) = s.view.conversations[ci].messages.last().filter(|m| {
                m.role == "assistant"
                    && m.extra.get("agentStatus") == Some(&json!("completed"))
                    && m.extra
                        .get("agentWorkspace")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        == workspace
                            .as_ref()
                            .and_then(|w| w.root.to_str())
                            .unwrap_or("")
            })
        {
            let chat = &s.view.conversations[ci];
            if let Some(objective) = previous
                .extra
                .get("agentObjective")
                .and_then(Value::as_str)
                .or_else(|| request_for(chat, previous).map(|m| m.content.as_str()))
            {
                pending_task = Some(crate::agentloop::intent::PendingTask {
                    id: previous.extra["agentTaskId"].as_str().unwrap_or("").into(),
                    objective: objective.into(),
                    contract: previous.extra.get("agentContract").cloned(),
                    completed: true,
                });
            }
        }
        if v["continueTaskId"].as_str().is_some() && parent.as_ref() != v.get("continueTaskId") {
            return Err("That task cannot be continued here. Use the same chat and workspace; denied tasks remain denied.".into());
        }
        let mut submitted_chat = ci.map(|i| s.view.conversations[i].clone());
        let mut history = vec![];
        if let Some(chat) = &mut submitted_chat {
            if task.is_empty() && chat.draft_images.is_empty() {
                return Err("Write a message or attach an image.".into());
            }
            let mut message = Message::new("user", task.into());
            message.attachments = std::mem::take(&mut chat.draft_images);
            chat.messages.push(message);
            history = self.repository.wire_messages(chat)?;
            if parent.is_some()
                && let Some(message) = history.last_mut()
            {
                message["content"] = json!(format!(
                    "Continue the pending request. {effective_task}\nDetermine whether the latest user message answers, corrects, cancels, or replaces the pending task; do not assume it is an answer. Choices never grant execution permission."
                ));
            }
        }
        let id = self.agent.submit_continuation(
            Submission {
                owner: session.into(),
                model: Some(model.clone()),
                task: effective_task.clone(),
                workspace: workspace
                    .as_ref()
                    .map(|w| w.root.clone())
                    .unwrap_or_default(),
                limits,
                conversation_id: ci.map(|i| s.view.conversations[i].id.clone()),
                request_id: request.clone(),
                history,
            },
            Arc::new(CoreInference {
                core: self.clone(),
                session: session.into(),
                model,
                effort: serde_json::from_value(v.get("effort").cloned().unwrap_or(json!("off")))
                    .map_err(|e| format!("Invalid thinking effort: {e}"))?,
            }),
            if workspace.is_some() {
                Registry::default()
            } else {
                Registry::empty()
            },
            resume_evidence,
            pending_task,
        )?;
        if let Some(ci) = ci {
            let mut chat = submitted_chat.take().unwrap();
            let message = chat.messages.last_mut().unwrap();
            message.extra.extend([
                ("agentTaskId".into(), json!(id)),
                ("agentRequestId".into(), json!(request)),
                ("agentKind".into(), json!("request")),
                ("agentParent".into(), json!(parent)),
                ("agentObjective".into(), json!(effective_task)),
                (
                    "agentWorkspace".into(),
                    json!(workspace.as_ref().map(|w| &w.root)),
                ),
            ]);
            if chat.messages.len() == 1 && chat.title == "New chat" {
                chat.title = task.chars().take(60).collect();
            }
            if chat.draft.trim() == task {
                chat.draft.clear();
            }
            chat.updated_at = now();
            if let Err(e) = self.repository.save(&chat) {
                let _ = self.agent.cancel(session, &id);
                return Err(format!("Couldn’t save the agent request: {e}"));
            }
            s.view.conversations[ci] = chat;
            self.publish(&mut s);
        }
        Ok(json!({"taskId":id}))
    }
    pub(super) fn save_agent_result(&self) {
        let Some(task) = self.agent.snapshot() else {
            return;
        };
        let Some(chat_id) = task.conversation_id else {
            return;
        };
        let mut s = self.state.lock().unwrap();
        let Ok(ci) = Self::chat_index(&s, &chat_id) else {
            return;
        };
        let mut chat = s.view.conversations[ci].clone();
        if let Some(old) = task
            .contract
            .as_ref()
            .and_then(|c| c["supersedesTaskId"].as_str())
        {
            for message in &mut chat.messages {
                if message.role == "assistant"
                    && message.extra.get("agentTaskId").and_then(Value::as_str) == Some(old)
                {
                    message
                        .extra
                        .insert("agentStatus".into(), json!("cancelled"));
                    message
                        .extra
                        .insert("agentSupersededBy".into(), json!(task.id));
                }
                if message.role == "user"
                    && message.extra.get("agentTaskId").and_then(Value::as_str) == Some(&task.id)
                {
                    message.extra.insert("agentParent".into(), Value::Null);
                }
            }
        }
        let existing = chat.messages.iter().position(|m| {
            m.role == "assistant"
                && m.extra.get("agentTaskId").and_then(Value::as_str) == Some(&task.id)
        });
        if let Some(index) = existing
            && chat.messages[index]
                .extra
                .get("agentStatus")
                .is_some_and(|v| {
                    matches!(
                        v.as_str(),
                        Some(
                            "completed"
                                | "cancelled"
                                | "failed"
                                | "incomplete"
                                | "denied"
                                | "waiting_for_input"
                        )
                    )
                })
        {
            return;
        }
        if !chat.messages.iter().any(|m| {
            m.role == "user" && m.extra.get("agentTaskId").and_then(Value::as_str) == Some(&task.id)
        }) {
            return;
        }
        let mut content = task.assistant_text.trim_end().to_string();
        if !task.status.active() && task.status != Status::Completed {
            if !content.is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(task.result.as_deref().unwrap_or("Interaction interrupted."));
        } else if content.is_empty() && !task.status.active() {
            content = task.result.clone().unwrap_or_default();
        }
        let mut response = existing
            .map(|i| chat.messages[i].clone())
            .unwrap_or_else(|| Message::new("assistant", String::new()));
        response.content = content;
        response.extra.extend([
            ("agentTaskId".into(), json!(task.id)),
            ("agentKind".into(), json!("result")),
            ("agentStatus".into(), json!(task.status)),
            ("agentHistory".into(), json!(task.transcript)),
            ("agentEvents".into(), json!(task.audit)),
            ("agentUsage".into(), json!(task.usage)),
            ("agentModel".into(), json!(task.model)),
            (
                "agentWorkspace".into(),
                if task.workspace.is_empty() {
                    Value::Null
                } else {
                    json!(task.workspace)
                },
            ),
            ("agentObjective".into(), json!(task.task)),
            ("agentContract".into(), json!(task.contract)),
            (
                "agentParent".into(),
                chat.messages
                    .iter()
                    .find(|m| {
                        m.role == "user" && m.extra.get("agentTaskId") == Some(&json!(task.id))
                    })
                    .and_then(|m| m.extra.get("agentParent"))
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
        ]);
        if existing.is_some_and(|i| {
            chat.messages[i].content == response.content && chat.messages[i].extra == response.extra
        }) {
            return;
        }
        if let Some(i) = existing {
            chat.messages[i] = response;
        } else {
            chat.messages.push(response);
        }
        if let Ok(si) = Self::session_index(&s, &task.owner) {
            s.view.sessions[si].usage = task.usage.clone();
        }
        chat.updated_at = now();
        match self.repository.save(&chat) {
            Ok(()) => {
                s.view.conversations[ci] = chat;
                s.dirty.remove(&chat_id);
                self.publish(&mut s);
            }
            Err(e) => {
                s.view.warning = Some(format!("Agent result could not be saved: {e}"));
                self.publish(&mut s);
            }
        }
    }
    pub(super) fn recover_agent_chats(
        repository: &Repository,
        chats: &mut [Conversation],
    ) -> Result<()> {
        for chat in chats {
            let unfinished: Vec<_> = chat
                .messages
                .iter()
                .filter(|m| m.extra.get("agentKind") == Some(&json!("request")))
                .filter_map(|m| m.extra.get("agentTaskId").and_then(Value::as_str))
                .filter(|id| {
                    !chat.messages.iter().any(|m| {
                        m.role == "assistant"
                            && m.extra.get("agentTaskId").and_then(Value::as_str) == Some(id)
                            && m.extra.get("agentStatus").is_some_and(|s| {
                                matches!(
                                    s.as_str(),
                                    Some(
                                        "completed"
                                            | "cancelled"
                                            | "failed"
                                            | "incomplete"
                                            | "denied"
                                            | "waiting_for_input"
                                    )
                                )
                            })
                    })
                })
                .map(String::from)
                .collect();
            for id in &unfinished {
                let mut response=Message::new("assistant","Agent task interrupted when the Rust backend stopped. It was not resumed. Any completed approved file changes are retained.".into());
                response.extra.extend([
                    ("agentTaskId".into(), json!(id)),
                    ("agentKind".into(), json!("result")),
                    ("agentStatus".into(), json!(Status::Cancelled)),
                ]);
                if let Some(existing) = chat.messages.iter_mut().find(|m| {
                    m.role == "assistant"
                        && m.extra.get("agentTaskId").and_then(Value::as_str) == Some(id)
                }) {
                    existing.content.push_str("\n\n");
                    existing.content.push_str(&response.content);
                    existing
                        .extra
                        .insert("agentStatus".into(), json!(Status::Cancelled));
                } else {
                    chat.messages.push(response);
                }
            }
            if !unfinished.is_empty() {
                chat.updated_at = now();
                repository.save(chat).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}

// Follow persisted execution IDs, never model-supplied checkpoint claims. Each
// transcript is stored once; continuations reference it rather than copying it.
fn linked_evidence(chat: &Conversation, id: &str) -> Result<Vec<Value>> {
    let mut id = id.to_owned();
    let mut seen = std::collections::HashSet::new();
    let mut groups = Vec::new();
    while !id.is_empty() {
        if !seen.insert(id.clone()) {
            return Err("Invalid continuation chain".into());
        }
        let message = chat
            .messages
            .iter()
            .find(|m| m.role == "assistant" && m.extra.get("agentTaskId") == Some(&json!(id)))
            .ok_or("Continuation records are missing")?;
        groups.push(
            message
                .extra
                .get("agentHistory")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        );
        id = message
            .extra
            .get("agentParent")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
    }
    Ok(groups.into_iter().rev().flatten().collect())
}

fn request_for<'a>(chat: &'a Conversation, response: &Message) -> Option<&'a Message> {
    chat.messages.iter().find(|m| {
        m.role == "user" && m.extra.get("agentTaskId") == response.extra.get("agentTaskId")
    })
}
