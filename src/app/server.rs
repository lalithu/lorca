//! Compatibility boundary for the existing macOS chat UI. No task-language classifier.
use crate::{
    inference::*,
    permissions::Permissions,
    runtime::{Runtime, Submission},
    storage::{Repository, valid_id},
    types::*,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::watch;
struct Client {
    last: Instant,
    explicit: Option<ModelRoute>,
}
struct State {
    view: Snapshot,
    clients: HashMap<String, Client>,
    task_revision: u64,
}
pub struct Server {
    pub repository: Repository,
    state: Mutex<State>,
    changes: watch::Sender<Snapshot>,
    runtime: Arc<Runtime>,
    http: reqwest::Client,
}
fn text<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    crate::tools::text(v, k)
}
impl Server {
    pub fn new(repository: Repository) -> Result<Arc<Self>> {
        let (mut chats, _, warning) = repository.load().map_err(|e| e.to_string())?;
        // A restarted backend never resumes or replays an action.
        for chat in &mut chats {
            for m in &mut chat.messages {
                if m.extra
                    .get("agentStatus")
                    .and_then(Value::as_str)
                    .is_some_and(|s| matches!(s, "running" | "awaiting_approval"))
                {
                    m.extra.insert("agentStatus".into(), json!("incomplete"));
                    m.extra.insert("agentGenerating".into(), json!(false));
                    if m.content.is_empty() {
                        m.content="Interrupted by backend restart. Inspect existing results before continuing.".into();
                    }
                }
            }
        }
        if chats.is_empty() {
            let c = Conversation::default();
            repository.save(&c).map_err(|e| e.to_string())?;
            chats.push(c)
        }
        let roster = super::roster::Roster::load(&repository.root, &chats);
        let view = Snapshot {
            roster,
            conversations: chats,
            warning,
            ..Default::default()
        };
        let (changes, _) = watch::channel(view.clone());
        Ok(Arc::new(Self {
            repository,
            state: Mutex::new(State {
                view,
                clients: HashMap::new(),
                task_revision: 0,
            }),
            changes,
            runtime: Arc::new(Runtime::default()),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(3))
                .build()
                .map_err(|e| e.to_string())?,
        }))
    }
    pub fn agent(&self) -> &Arc<Runtime> {
        &self.runtime
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().unwrap().view.clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<Snapshot> {
        self.changes.subscribe()
    }
    fn publish(&self, s: &mut State) {
        s.view.revision += 1;
        self.changes.send_replace(s.view.clone());
    }
    pub fn touch(&self, id: &str) -> Result<()> {
        self.state
            .lock()
            .unwrap()
            .clients
            .get_mut(id)
            .ok_or("Session expired; reconnect")?
            .last = Instant::now();
        Ok(())
    }
    pub fn client_count(&self) -> usize {
        self.state.lock().unwrap().clients.len()
    }
    pub fn maintenance(&self) {
        if let Err(e) = self.checkpoint() {
            let mut s = self.state.lock().unwrap();
            s.view.warning = Some(format!("Could not save chat: {e}"));
            self.publish(&mut s);
        }
        let expired: Vec<_> = self
            .state
            .lock()
            .unwrap()
            .clients
            .iter()
            .filter(|(_, c)| c.last.elapsed() > Duration::from_secs(60))
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.runtime.cancel_owner(&id);
            let mut s = self.state.lock().unwrap();
            s.clients.remove(&id);
            s.view.sessions.retain(|c| c.id != id);
            self.publish(&mut s);
        }
    }
    pub fn checkpoint(&self) -> Result<()> {
        let Some(t) = self.runtime.checkpoint() else {
            return Ok(());
        };
        let revision = t.view["revision"].as_u64().unwrap_or(0);
        let mut s = self.state.lock().unwrap();
        if s.task_revision == revision {
            return Ok(());
        }
        let cid = t.view["conversationId"]
            .as_str()
            .ok_or("Task has no conversation")?;
        let ci = s
            .view
            .conversations
            .iter()
            .position(|c| c.id == cid)
            .ok_or("Task conversation missing")?;
        let mut chat = s.view.conversations[ci].clone();
        let task_id = t.view["id"].as_str().unwrap();
        let m = chat
            .messages
            .iter_mut()
            .find(|m| m.role == "assistant" && m.extra.get("agentTaskId") == Some(&json!(task_id)))
            .ok_or("Task message missing")?;
        m.content = t.view["result"]
            .as_str()
            .or(t.view["assistantText"].as_str())
            .unwrap_or("")
            .into();
        for (key, value) in [
            ("agentKind", json!("result")),
            ("agentStatus", t.view["status"].clone()),
            ("agentEvents", t.view["events"].clone()),
            ("agentHistory", json!(t.history)),
            ("agentUsage", t.view["usage"].clone()),
            ("agentModel", t.view["model"].clone()),
            ("agentGenerating", t.view["generating"].clone()),
        ] {
            m.extra.insert(key.into(), value);
        }
        chat.extra
            .insert("runtimePermissions".into(), json!(t.permissions));
        chat.updated_at = now();
        self.repository.save(&chat).map_err(|e| e.to_string())?;
        let mut roster = s.view.roster.clone();
        if roster.observe_chat(&chat) {
            if let Err(error) = roster.save(&self.repository.root) {
                roster.error = Some(error);
            }
            s.view.roster = roster;
        }
        s.view.conversations[ci] = chat;
        s.task_revision = revision;
        let usage: Usage = serde_json::from_value(t.view["usage"].clone()).unwrap_or_default();
        if let Some(session) = s.view.sessions.iter_mut().find(|c| c.id == t.view["owner"]) {
            session.usage = usage.clone();
        }
        self.publish(&mut s);
        Ok(())
    }
    pub fn loading(&self, session: &str, model: Option<&str>, capacity: Option<u64>) {
        let mut s = self.state.lock().unwrap();
        if let Some(c) = s.view.sessions.iter_mut().find(|c| c.id == session) {
            c.loading_model = model.map(str::to_owned);
            if model.is_none() {
                if let Some(n) = capacity {
                    c.allocated_context = Some(n);
                    c.usage.context_limit = Some(n);
                }
                c.active_model = Some(c.model.clone());
            }
        }
        self.publish(&mut s);
    }
    async fn refresh(&self, session: &str) -> Result<()> {
        let explicit = self
            .state
            .lock()
            .unwrap()
            .clients
            .get(session)
            .and_then(|c| c.explicit.clone());
        let routes = explicit.map(|r| vec![r]).unwrap_or_else(|| {
            vec![
                ModelRoute {
                    provider: Engine::Ollama,
                    host: "http://127.0.0.1:11434".into(),
                },
                ModelRoute {
                    provider: Engine::LlamaCpp,
                    host: "http://127.0.0.1:9931".into(),
                },
                ModelRoute {
                    provider: Engine::LlamaCpp,
                    host: "http://127.0.0.1:8080".into(),
                },
            ]
        });
        let results = futures_util::future::join_all(
            routes
                .into_iter()
                .map(|r| crate::providers::catalog(&self.http, r)),
        )
        .await;
        let mut choices: Vec<_> = results
            .into_iter()
            .filter_map(std::result::Result::ok)
            .flatten()
            .collect();
        // Discover advertised maxima, not pretend these are allocations.
        for c in &mut choices {
            if c.route.provider == Engine::Ollama
                && let Ok(r) = self
                    .http
                    .post(format!("{}/api/show", c.route.host))
                    .json(&json!({"model":c.model}))
                    .timeout(Duration::from_secs(2))
                    .send()
                    .await
                && let Ok(v) = r.json::<Value>().await
            {
                c.context_limit = v["model_info"].as_object().and_then(|o| {
                    o.iter()
                        .find(|(k, _)| k.ends_with(".context_length"))
                        .and_then(|(_, v)| v.as_u64())
                });
            }
        }
        let mut s = self.state.lock().unwrap();
        let c = s
            .view
            .sessions
            .iter_mut()
            .find(|c| c.id == session)
            .ok_or("Session expired")?;
        let choice = c
            .model_choice
            .as_ref()
            .and_then(|old| {
                choices
                    .iter()
                    .find(|n| n.route == old.route && n.model == old.model)
            })
            .or_else(|| choices.iter().find(|n| n.model == c.model))
            .or(choices.first())
            .cloned();
        c.models = choices.iter().map(|c| c.model.clone()).collect();
        c.model_choices = choices;
        c.is_connected = choice.is_some();
        if let Some(choice) = choice {
            c.provider = choice.route.provider.label().into();
            c.model = choice.model.clone();
            c.model_choice = Some(choice);
            c.error = None;
        } else {
            c.model_choice = None;
            c.error =
                Some("Start Ollama or llama.cpp, install a model, then refresh models.".into());
        }
        self.publish(&mut s);
        Ok(())
    }
    fn selected_model(&self, session: &str, v: &Value) -> Result<Model> {
        let s = self.state.lock().unwrap();
        let c = s
            .view
            .sessions
            .iter()
            .find(|c| c.id == session)
            .ok_or("Session expired")?;
        let choice = c
            .model_choice
            .clone()
            .ok_or("Start Ollama or llama.cpp and select a model")?;
        if v["model"].as_str().is_some_and(|m| m != choice.model) {
            return Err("Selected model changed; send again".into());
        }
        let requested = if v["configureContext"] == true {
            v["contextWindow"]
                .as_u64()
                .filter(|n| *n >= 1024 && *n <= 1048576)
        } else {
            None
        };
        Model::new(
            choice,
            requested,
            v["effort"].as_str().unwrap_or("off").into(),
        )
    }
    fn start(self: &Arc<Self>, session: &str, v: &Value) -> Result<Value> {
        let task = text(v, "task")
            .or_else(|_| text(v, "text"))
            .unwrap_or("")
            .trim()
            .to_owned();
        let model = self.selected_model(session, v)?;
        let cid = valid_id(text(v, "conversationId")?).map_err(|e| e.to_string())?;
        let rid = valid_id(text(v, "requestId")?).map_err(|e| e.to_string())?;
        {
            let s = self.state.lock().unwrap();
            let chat = s
                .view
                .conversations
                .iter()
                .find(|c| c.id == cid)
                .ok_or("Unknown chat")?;
            if let Some(m) = chat.messages.iter().find(|m| {
                m.extra.get("agentRequestId") == Some(&json!(rid))
                    && m.extra.get("agentTaskId").is_some_and(Value::is_string)
            }) {
                return Ok(json!({"taskId":m.extra["agentTaskId"],"alreadyAccepted":true}));
            }
        }
        if self.runtime.active() {
            return Err(
                "A task is running. Use Correct task or stop it before sending another request."
                    .into(),
            );
        }
        let selected = v["workspace"].as_str().filter(|s| !s.is_empty());
        let cwd = match selected {
            Some(p) => std::path::PathBuf::from(p),
            None => std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .ok_or("HOME is unavailable")?,
        }
        .canonicalize()
        .map_err(|e| e.to_string())?;
        if !cwd.is_dir() {
            return Err("Working directory must be a folder".into());
        }
        let mut s = self.state.lock().unwrap();
        let ci = s
            .view
            .conversations
            .iter()
            .position(|c| c.id == cid)
            .ok_or("Unknown chat")?;
        let mut chat = s.view.conversations[ci].clone();
        if task.is_empty() && chat.draft_images.is_empty() {
            return Err("Write a message or attach an image".into());
        }
        let mut user = Message::new("user", task.clone());
        user.attachments = std::mem::take(&mut chat.draft_images);
        user.extra.insert("agentRequestId".into(), json!(rid));
        chat.messages.push(user);
        if chat.title == "New chat" {
            chat.title = if task.is_empty() {
                "Image chat".into()
            } else {
                task.chars().take(60).collect()
            }
        }
        chat.draft.clear();
        let history = self.repository.wire_messages(&chat)?;
        let permissions = chat
            .extra
            .get("runtimePermissions")
            .cloned()
            .and_then(|v| serde_json::from_value::<Permissions>(v).ok())
            .unwrap_or_default();
        // Hold application state until the placeholder exists; checkpoints cannot observe a half-accepted task.
        let task_id = self.runtime.start(
            self.clone(),
            Submission {
                owner: session.into(),
                conversation: cid,
                request: rid.clone(),
                task: task.clone(),
                cwd,
                selected_directory: selected.is_some(),
                model,
                history,
                permissions,
            },
        )?;
        if let Some(user) = chat.messages.last_mut() {
            user.extra.insert("agentTaskId".into(), json!(task_id));
            user.extra
                .insert("agentWorkspace".into(), json!(selected.unwrap_or("")));
        }
        let mut answer = Message::new("assistant", String::new());
        answer.extra.insert("agentTaskId".into(), json!(task_id));
        answer.extra.insert("agentRequestId".into(), json!(rid));
        answer.extra.insert("agentStatus".into(), json!("running"));
        answer.extra.insert("agentKind".into(), json!("result"));
        chat.messages.push(answer);
        if let Err(e) = self.repository.save(&chat) {
            self.runtime.cancel_owner(session);
            return Err(e.to_string());
        }
        s.view.conversations[ci] = chat;
        s.task_revision = 0;
        self.publish(&mut s);
        Ok(json!({"taskId":task_id}))
    }
    fn roster_add(&self, v: &Value) -> Result<Value> {
        let objective = text(v, "objective")?.trim();
        if objective.is_empty() {
            return Err("Write a task first".into());
        }
        let task_id = valid_id(text(v, "taskId")?).map_err(|e| e.to_string())?;
        let mut s = self.state.lock().unwrap();
        let mut roster = s.view.roster.clone();
        if let Some(existing) = roster.tasks.iter().find(|t| t.id == task_id) {
            if existing.objective != objective {
                return Err("Task identifier already used".into());
            }
            return Ok(json!(s.view));
        }
        let mut task = super::roster::Task::new(objective.into());
        task.id = task_id;
        roster.tasks.insert(0, task);
        roster.save(&self.repository.root)?;
        s.view.roster = roster;
        self.publish(&mut s);
        Ok(json!(s.view))
    }
    fn roster_run(self: &Arc<Self>, session: &str, v: &Value) -> Result<Value> {
        let task_id = valid_id(text(v, "taskId")?).map_err(|e| e.to_string())?;
        self.checkpoint()?;
        let (cid, objective) = {
            let mut s = self.state.lock().unwrap();
            let mut roster = s.view.roster.clone();
            let task = roster
                .tasks
                .iter_mut()
                .find(|t| t.id == task_id)
                .ok_or("Unknown roster task")?;
            // A repeated request only returns the existing work; it cannot replay it.
            if task.conversation_id.is_some() {
                return Ok(json!(s.view));
            }
            if self.runtime.active() {
                return Err("Finish or stop the active task before starting roster work".into());
            }
            if let Some(error) = &roster.error {
                return Err(error.clone());
            }
            let task = roster.tasks.iter_mut().find(|t| t.id == task_id).unwrap();
            let mut chat = Conversation::default();
            chat.title = task.objective.chars().take(60).collect();
            self.repository.save(&chat).map_err(|e| e.to_string())?;
            task.conversation_id = Some(chat.id.clone());
            task.status = "incomplete".into();
            task.updated_at = now();
            let result = (chat.id.clone(), task.objective.clone());
            roster.save(&self.repository.root)?;
            s.view.roster = roster;
            s.view.conversations.push(chat);
            self.publish(&mut s);
            result
        };
        let mut request = v.clone();
        request["conversationId"] = json!(cid);
        request["requestId"] = json!(task_id);
        request["task"] = json!(objective);
        // start() remains the single execution path, including all existing gates.
        if let Err(error) = self.start(session, &request) {
            let mut s = self.state.lock().unwrap();
            let mut roster = s.view.roster.clone();
            if let Some(task) = roster.tasks.iter_mut().find(|t| t.id == task_id) {
                task.status = "failed".into();
                task.summary = Some(error.clone());
            }
            if let Err(save_error) = roster.save(&self.repository.root) {
                roster.error = Some(save_error);
            }
            s.view.roster = roster;
            self.publish(&mut s);
            return Err(error);
        }
        self.checkpoint()?;
        Ok(json!(self.snapshot()))
    }
    pub async fn command(self: &Arc<Self>, v: Value) -> Result<Value> {
        let op = text(&v, "op")?;
        if op == "journal_location" {
            if let Some(path) = v["path"].as_str() {
                self.repository
                    .set_journal_root(path.into())
                    .map_err(|e| e.to_string())?;
            }
            let path = self.repository.journal_root().map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            return Ok(json!({"path":path}));
        }
        if op == "connect" {
            let sid = {
                let mut s = self.state.lock().unwrap();
                let selected = self
                    .repository
                    .load()
                    .ok()
                    .and_then(|(_, id, _)| id)
                    .filter(|id| s.view.conversations.iter().any(|c| &c.id == id))
                    .unwrap_or_else(|| s.view.conversations[0].id.clone());
                let sid = id();
                let provider = if v["provider"] == "llama_cpp" {
                    Engine::LlamaCpp
                } else {
                    Engine::Ollama
                };
                let explicit = if v["provider"] == "auto" || v["host"].is_null() {
                    None
                } else {
                    let host = text(&v, "host")?.trim_end_matches('/');
                    let u = reqwest::Url::parse(host).map_err(|e| e.to_string())?;
                    if !["http", "https"].contains(&u.scheme()) || u.host_str().is_none() {
                        return Err("Invalid model server address".into());
                    }
                    Some(ModelRoute {
                        provider,
                        host: host.into(),
                    })
                };
                s.clients.insert(
                    sid.clone(),
                    Client {
                        last: Instant::now(),
                        explicit,
                    },
                );
                s.view.sessions.push(SessionView {
                    id: sid.clone(),
                    selected_id: selected,
                    provider: provider.label().into(),
                    model: v["model"].as_str().unwrap_or("").into(),
                    loading_model: None,
                    active_model: None,
                    models_to_release: vec![],
                    models: vec![],
                    model_choices: vec![],
                    model_choice: serde_json::from_value(v["preferredModel"].clone()).ok(),
                    is_connected: false,
                    error: None,
                    allocated_context: None,
                    usage: Usage::default(),
                });
                self.publish(&mut s);
                sid
            };
            self.refresh(&sid).await?;
            return Ok(json!({"session":sid,"state":self.snapshot()}));
        }
        let session = text(&v, "session")?;
        self.touch(session)?;
        match op {
            "snapshot" | "heartbeat" => {
                self.checkpoint()?;
                return Ok(json!(self.snapshot()));
            }
            "agent_start" | "send" => return self.start(session, &v),
            "roster_add" => return self.roster_add(&v),
            "roster_run" => {
                // Validate the selected local model before saving an execution reference.
                self.selected_model(session, &v)?;
                return self.roster_run(session, &v);
            }
            "exportConversation" => {
                self.checkpoint()?;
                let s = self.state.lock().unwrap();
                let cid = text(&v, "conversationId")?;
                let chat = s
                    .view
                    .conversations
                    .iter()
                    .find(|c| c.id == cid)
                    .ok_or("Unknown chat")?;
                let format = v["format"].as_str().unwrap_or("markdown");
                let content = match format {
                    "json" => serde_json::to_string_pretty(chat).map_err(|e| e.to_string())?,
                    "markdown" => {
                        let mut out = format!("# {}\n\n", chat.title);
                        for m in &chat.messages {
                            out.push_str(&format!("## {}\n\n{}\n\n", m.role, m.content));
                            if let Some(h) = m.extra.get("agentHistory") {
                                out.push_str(&format!(
                                    "### Tool records\n\n```json\n{}\n```\n\n",
                                    serde_json::to_string_pretty(h).unwrap()
                                ));
                            }
                        }
                        out
                    }
                    _ => return Err("Choose json or markdown".into()),
                };
                return Ok(
                    json!({"format":format,"content":content,"filename":format!("lorca-{}.{}",chat.id,if format=="json"{"json"}else{"md"})}),
                );
            }
            "agent_snapshot" => return Ok(json!(self.runtime.snapshot())),
            "agent_decide" => {
                self.runtime.decide(
                    session,
                    text(&v, "taskId")?,
                    text(&v, "approvalId")?,
                    text(&v, "decision")?,
                )?;
                return Ok(json!({"accepted":true}));
            }
            "agent_cancel" => {
                self.runtime.cancel(session, text(&v, "taskId")?)?;
                return Ok(json!({"accepted":true}));
            }
            "agent_status" => {
                let t = self.runtime.snapshot().ok_or("No task")?;
                if t["owner"] != session {
                    return Err("Task belongs to another session".into());
                }
                return Ok(
                    json!({"taskId":t["id"],"status":t["status"],"task":t["task"],"revision":t["revision"]}),
                );
            }
            "agent_revise" => {
                let task = text(&v, "task")?;
                self.runtime.revise(session, text(&v, "taskId")?, task)?;
                let mut s = self.state.lock().unwrap();
                let cid = self.runtime.snapshot().unwrap()["conversationId"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                let c = s
                    .view
                    .conversations
                    .iter_mut()
                    .find(|c| c.id == cid)
                    .unwrap();
                c.messages.push(Message::new("user", task.into()));
                self.repository.save(c).map_err(|e| e.to_string())?;
                self.publish(&mut s);
                return Ok(json!({"accepted":true,"taskId":v["taskId"]}));
            }
            "refresh" => self.refresh(session).await?,
            "prepare" | "release" => {
                if self.runtime.active() {
                    return Err("Stop active work before loading or unloading a model".into());
                }
                let m = self.selected_model(session, &v)?;
                let load = op == "prepare";
                self.loading(session, load.then_some(m.choice.model.as_str()), None);
                let result = crate::providers::prepare(&m, load).await;
                self.loading(session, None, result.as_ref().ok().copied().flatten());
                result?;
                if !load {
                    let mut s = self.state.lock().unwrap();
                    if let Some(c) = s.view.sessions.iter_mut().find(|c| c.id == session) {
                        c.active_model = None;
                        c.allocated_context = None;
                        c.usage = Usage::default();
                    }
                    self.publish(&mut s);
                    return Ok(json!({"unloaded":true}));
                }
            }
            "cancelLoad" | "cancel" => {
                self.runtime.cancel_owner(session);
            }
            "close" => {
                self.runtime.cancel_owner(session);
                self.checkpoint()?;
                if v["unload"] == true {
                    let m = self.selected_model(session, &json!({}))?;
                    crate::providers::prepare(&m, false).await?;
                }
                let mut s = self.state.lock().unwrap();
                s.clients.remove(session);
                s.view.sessions.retain(|c| c.id != session);
                self.publish(&mut s);
                return Ok(json!({"closed":true}));
            }
            "notes_cancel" => return Ok(json!({"cancelled":true})),
            "notes_follow" | "hardwareAdvice" => {
                return Err(
                    "This feature is deferred while the main chat runtime is being replaced."
                        .into(),
                );
            }
            _ => self.edit(session, op, &v)?,
        }
        Ok(json!(self.snapshot()))
    }
    fn edit(&self, session: &str, op: &str, v: &Value) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        let si = s
            .view
            .sessions
            .iter()
            .position(|c| c.id == session)
            .ok_or("Session expired")?;
        let cid = v["conversationId"]
            .as_str()
            .unwrap_or(&s.view.sessions[si].selected_id)
            .to_owned();
        match op {
            "selectModel" => {
                if self.runtime.active() {
                    return Err("Stop the active run before switching models".into());
                }
                let route: ModelRoute =
                    serde_json::from_value(v["route"].clone()).map_err(|e| e.to_string())?;
                let model = text(v, "model")?;
                let c = &mut s.view.sessions[si];
                let choice = c
                    .model_choices
                    .iter()
                    .find(|c| c.route == route && c.model == model)
                    .cloned()
                    .ok_or("Refresh and select an available model")?;
                c.model = model.into();
                c.provider = route.provider.label().into();
                c.model_choice = Some(choice);
                c.allocated_context = None;
                c.active_model = None;
                c.usage = Usage::default();
            }
            "new" => {
                let c = Conversation::default();
                self.repository.save(&c).map_err(|e| e.to_string())?;
                self.repository.select(&c.id).map_err(|e| e.to_string())?;
                s.view.sessions[si].selected_id = c.id.clone();
                s.view.sessions[si].usage = Usage::default();
                s.view.conversations.push(c);
            }
            "open" => {
                if !s.view.conversations.iter().any(|c| c.id == cid) {
                    return Err("Unknown chat".into());
                }
                self.repository.select(&cid).map_err(|e| e.to_string())?;
                s.view.sessions[si].selected_id = cid;
            }
            "delete" => {
                if self.runtime.active()
                    && self
                        .runtime
                        .snapshot()
                        .is_some_and(|t| t["conversationId"] == cid)
                {
                    return Err("Stop the run before deleting its chat".into());
                }
                self.repository.delete(&cid).map_err(|e| e.to_string())?;
                s.view.conversations.retain(|c| c.id != cid);
                if s.view.conversations.is_empty() {
                    let c = Conversation::default();
                    self.repository.save(&c).map_err(|e| e.to_string())?;
                    s.view.conversations.push(c)
                }
                let next = s.view.conversations[0].id.clone();
                for c in &mut s.view.sessions {
                    if c.selected_id == cid {
                        c.selected_id = next.clone()
                    }
                }
            }
            "clearError" => s.view.sessions[si].error = None,
            "draft" | "rename" | "importImage" | "removeImage" | "setPermissions" => {
                let ci = s
                    .view
                    .conversations
                    .iter()
                    .position(|c| c.id == cid)
                    .ok_or("Unknown chat")?;
                let mut c = s.view.conversations[ci].clone();
                match op {
                    "draft" => c.draft = text(v, "text").unwrap_or("").into(),
                    "rename" => c.title = text(v, "title")?.chars().take(120).collect(),
                    "importImage" => {
                        if c.draft_images.len() >= 4 {
                            return Err("Attach up to four images".into());
                        }
                        c.draft_images.push(
                            self.repository
                                .import_image(text(v, "data")?, text(v, "name")?)?,
                        );
                    }
                    "removeImage" => c.draft_images.retain(|a| a.id != v["imageId"]),
                    "setPermissions" => {
                        if self.runtime.active() {
                            return Err("Stop the run before changing permissions".into());
                        }
                        let p: Permissions = serde_json::from_value(v["permissions"].clone())
                            .map_err(|e| e.to_string())?;
                        c.extra.insert("runtimePermissions".into(), json!(p));
                    }
                    _ => {}
                }
                c.updated_at = now();
                self.repository.save(&c).map_err(|e| e.to_string())?;
                s.view.conversations[ci] = c;
            }
            _ => return Err(format!("Unsupported operation {op}")),
        }
        self.publish(&mut s);
        Ok(())
    }
}
