mod agent;
mod conversations;
mod generation;
mod models;
use crate::{
    provider::{self, Update},
    storage::Repository,
    types::*,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{Mutex as AsyncMutex, watch};

type Result<T> = std::result::Result<T, String>;
struct ClientSession {
    host: String,
    last_seen: Instant,
}
struct State {
    view: Snapshot,
    clients: HashMap<String, ClientSession>,
    selected: Option<String>,
    dirty: HashSet<String>,
    cancels: HashMap<String, watch::Sender<bool>>,
    loads: HashMap<String, watch::Sender<bool>>,
}
pub struct Core {
    agent: Arc<crate::agent::Agent>,
    pub repository: Repository,
    state: Mutex<State>,
    changes: watch::Sender<Snapshot>,
    client: reqwest::Client,
    lifecycle: AsyncMutex<()>,
}
impl Core {
    pub fn new(repository: Repository) -> Result<Arc<Self>> {
        let (mut conversations, selected, warning) =
            repository.load().map_err(|e| e.to_string())?;
        if conversations.is_empty() {
            let c = Conversation::default();
            repository.save(&c).map_err(|e| e.to_string())?;
            conversations.push(c);
        }
        let view = Snapshot {
            conversations,
            warning,
            ..Default::default()
        };
        let (changes, _) = watch::channel(view.clone());
        Ok(Arc::new(Self {
            agent: Arc::new(crate::agent::Agent::default()),
            repository,
            state: Mutex::new(State {
                view,
                clients: HashMap::new(),
                selected,
                dirty: HashSet::new(),
                cancels: HashMap::new(),
                loads: HashMap::new(),
            }),
            changes,
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .build()
                .map_err(|e| e.to_string())?,
            lifecycle: AsyncMutex::new(()),
        }))
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
    pub fn touch(&self, session: &str) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        s.clients
            .get_mut(session)
            .ok_or("Session expired; reconnect to lorca.")?
            .last_seen = Instant::now();
        Ok(())
    }
    pub fn client_count(&self) -> usize {
        self.state.lock().unwrap().clients.len()
    }
    pub fn maintenance(&self) {
        let mut s = self.state.lock().unwrap();
        let stale: Vec<_> = s
            .clients
            .iter()
            .filter(|(_, c)| c.last_seen.elapsed() > Duration::from_secs(60))
            .map(|(id, _)| id.clone())
            .collect();
        let changed = !stale.is_empty() || !s.dirty.is_empty();
        for id in stale {
            self.agent.cancel_owner(&id);
            Self::cancel_session(&mut s, &id);
            s.clients.remove(&id);
            s.view.sessions.retain(|v| v.id != id);
        }
        let dirty: Vec<_> = s.dirty.iter().cloned().collect();
        for id in dirty {
            if let Some(c) = s.view.conversations.iter().find(|c| c.id == id) {
                match self.repository.save(c) {
                    Ok(()) => {
                        s.dirty.remove(&id);
                    }
                    Err(e) => {
                        s.view.warning = Some(format!("Chat changes couldn’t be saved: {e}"));
                    }
                }
            }
        }
        if changed {
            self.publish(&mut s);
        }
    }
    fn session_index(s: &State, id: &str) -> Result<usize> {
        s.view
            .sessions
            .iter()
            .position(|v| v.id == id)
            .ok_or("Session expired; reconnect to lorca.".into())
    }
    fn chat_index(s: &State, id: &str) -> Result<usize> {
        s.view
            .conversations
            .iter()
            .position(|c| c.id.eq_ignore_ascii_case(id))
            .ok_or("That conversation is no longer available.".into())
    }
    fn host(&self, session: &str) -> Result<String> {
        self.state
            .lock()
            .unwrap()
            .clients
            .get(session)
            .map(|c| c.host.clone())
            .ok_or("Session expired; reconnect.".into())
    }
    pub async fn command(self: &Arc<Self>, v: Value) -> Result<Value> {
        let op = text(&v, "op")?;
        if op == "connect" {
            let host = v["host"]
                .as_str()
                .unwrap_or("http://127.0.0.1:11434")
                .trim_end_matches('/')
                .to_string();
            let mut url = reqwest::Url::parse(&host).map_err(|_| "Invalid Ollama address")?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                return Err("Invalid Ollama address".into());
            }
            if url.host_str() == Some("localhost") {
                url.set_host(Some("127.0.0.1"))
                    .map_err(|_| "Invalid Ollama address")?;
            }
            let host = url.as_str().trim_end_matches('/').to_string();
            let mut s = self.state.lock().unwrap();
            let id = id();
            let selected = if v["fresh"].as_bool() == Some(true) {
                let c = Conversation::default();
                self.repository.save(&c).map_err(|e| e.to_string())?;
                let selected = c.id.clone();
                s.view.conversations.push(c);
                selected
            } else {
                s.selected
                    .clone()
                    .filter(|id| s.view.conversations.iter().any(|c| &c.id == id))
                    .unwrap_or_else(|| s.view.conversations[0].id.clone())
            };
            s.clients.insert(
                id.clone(),
                ClientSession {
                    host,
                    last_seen: Instant::now(),
                },
            );
            let context_used = s
                .view
                .conversations
                .iter()
                .find(|c| c.id == selected)
                .and_then(|c| c.messages.is_empty().then_some(0));
            s.view.sessions.push(SessionView {
                id: id.clone(),
                selected_id: selected,
                model: v["model"].as_str().unwrap_or("qwen3.5:9b").into(),
                loading_model: None,
                active_model: None,
                models_to_release: vec![],
                models: vec![],
                is_connected: false,
                error: None,
                usage: Usage {
                    context_used,
                    ..Default::default()
                },
            });
            self.publish(&mut s);
            return Ok(json!({"session":id,"state":s.view}));
        }
        let session = text(&v, "session")?;
        self.touch(session)?;
        if op.starts_with("agent_") {
            return self.agent_command(session, op, &v);
        }
        match op {
            "snapshot" | "heartbeat" => Ok(json!(self.snapshot())),
            "refresh" => self.refresh(session).await,
            "prepare" => {
                self.prepare(
                    session,
                    text(&v, "model")?,
                    v["unloadPrevious"].as_bool().unwrap_or(false),
                )
                .await
            }
            "release" => {
                self.release(session, text(&v, "model")?).await?;
                Ok(json!({"unloaded":true}))
            }
            "close" => {
                self.close(session, v["unload"].as_bool().unwrap_or(false))
                    .await
            }
            "cancelLoad" => {
                {
                    let s = self.state.lock().unwrap();
                    let matches = s
                        .view
                        .sessions
                        .iter()
                        .find(|s| s.id == session)
                        .is_some_and(|s| {
                            v["model"]
                                .as_str()
                                .is_none_or(|m| s.loading_model.as_deref() == Some(m))
                        });
                    if matches && let Some(cancel) = s.loads.get(session) {
                        cancel.send_replace(true);
                    }
                }
                Ok(json!(self.snapshot()))
            }
            "cancel" => {
                self.edit(session, op, &v)?;
                for _ in 0..200 {
                    let running = self.snapshot().turns.iter().any(|t| {
                        t.running
                            && v["conversationId"]
                                .as_str()
                                .is_some_and(|id| id == t.conversation_id)
                    });
                    if !running {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Ok(json!(self.snapshot()))
            }
            "send" => self.send(session, &v),
            _ => self.edit(session, op, &v),
        }
    }
}

pub fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key].as_str().ok_or_else(|| format!("Missing {key}"))
}
