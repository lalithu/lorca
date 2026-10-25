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
        let task = text(v, "task")?.trim();
        if task.is_empty() || task.len() > 8000 {
            return Err("Task must contain 1–8000 bytes".into());
        }
        let workspace = Workspace::new(std::path::Path::new(text(v, "workspace")?))?;
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
            if !s.view.conversations[ci].draft_images.is_empty() {
                return Err("Agent tasks are text-only. Switch to Chat to send these images; attachments have been kept.".into());
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
        let id = self.agent.submit(
            Submission {
                owner: session.into(),
                task: task.into(),
                workspace: workspace.root.clone(),
                limits,
                conversation_id: ci.map(|i| s.view.conversations[i].id.clone()),
                request_id: request.clone(),
            },
            Arc::new(CoreInference {
                core: self.clone(),
                session: session.into(),
                model,
            }),
            Registry::default(),
        )?;
        if let Some(ci) = ci {
            let mut chat = s.view.conversations[ci].clone();
            let mut message = Message::new("user", task.into());
            message.extra.extend([
                ("agentTaskId".into(), json!(id)),
                ("agentRequestId".into(), json!(request)),
                ("agentKind".into(), json!("request")),
                ("agentWorkspace".into(), json!(workspace.root)),
            ]);
            if chat.messages.is_empty() && chat.title == "New chat" {
                chat.title = task.chars().take(60).collect();
            }
            chat.messages.push(message);
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
        if task.status.active() {
            return;
        }
        let Some(chat_id) = task.conversation_id else {
            return;
        };
        let mut s = self.state.lock().unwrap();
        let Ok(ci) = Self::chat_index(&s, &chat_id) else {
            return;
        };
        let mut chat = s.view.conversations[ci].clone();
        if chat.messages.iter().any(|m| {
            m.role == "assistant"
                && m.extra.get("agentTaskId").and_then(Value::as_str) == Some(&task.id)
        }) {
            return;
        }
        if !chat.messages.iter().any(|m| {
            m.role == "user" && m.extra.get("agentTaskId").and_then(Value::as_str) == Some(&task.id)
        }) {
            return;
        }
        let mut response = Message::new(
            "assistant",
            task.result
                .unwrap_or_else(|| "Agent ended without a result.".into()),
        );
        response.extra.extend([
            ("agentTaskId".into(), json!(task.id)),
            ("agentKind".into(), json!("result")),
            ("agentStatus".into(), json!(task.status)),
        ]);
        chat.messages.push(response);
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
                chat.messages.push(response);
            }
            if !unfinished.is_empty() {
                chat.updated_at = now();
                repository.save(chat).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}
