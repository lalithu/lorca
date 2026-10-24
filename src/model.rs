// Terminal adapter. All model, conversation and lifecycle work runs in the shared core.
use crate::config::Config;
use lorca::client::Client;
pub use lorca::{provider::same_model, types::Usage};
use serde_json::json;
use std::{
    io,
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    time::Duration,
};
use tokio::{runtime::Runtime, sync::Mutex, task::JoinHandle};
type Link = Arc<Mutex<(Client, String)>>;
pub enum Update {
    Text(String),
    Done(Usage),
    Error(String),
}
pub struct Ollama {
    runtime: Runtime,
    link: Link,
    conversation: String,
    model: String,
    events: JoinHandle<()>,
    host: String,
}
pub struct Generation {
    pub updates: Receiver<Update>,
    task: JoinHandle<()>,
    link: Link,
    conversation: String,
    runtime: tokio::runtime::Handle,
    request_id: String,
}
pub struct ModelOperation {
    pub result: Receiver<Result<(), String>>,
    task: JoinHandle<()>,
    link: Link,
    model: String,
    load: bool,
    runtime: tokio::runtime::Handle,
}
impl Drop for ModelOperation {
    fn drop(&mut self) {
        if !self.task.is_finished() {
            self.task.abort();
            if self.load {
                let link = self.link.clone();
                let model = self.model.clone();
                self.runtime.spawn(async move {
                    let (client, session) = link.lock().await.clone();
                    let _ = client
                        .command(json!({"op":"cancelLoad","session":session,"model":model}))
                        .await;
                });
            }
        }
    }
}
impl Drop for Generation {
    fn drop(&mut self) {
        self.task.abort();
        let link = self.link.clone();
        let conversation = self.conversation.clone();
        let request = self.request_id.clone();
        self.runtime.spawn(async move {
            let (client,session)=link.lock().await.clone();
            let _ = client
                .command(json!({"op":"cancel","session":session,"conversationId":conversation,"requestId":request}))
                .await;
        });
    }
}
impl Ollama {
    pub fn new(config: &Config) -> io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()?;
        let (link, conversation, events) = runtime.block_on(async {
            let client = Client::connect(&std::env::current_exe()?).await?;
            let session = client
                .attach(&config.model, &config.base_url, true)
                .await
                .map_err(io::Error::other)?;
            let state = client
                .command(json!({"op":"snapshot","session":session}))
                .await
                .map_err(io::Error::other)?;
            let conversation = state["sessions"]
                .as_array()
                .and_then(|a| a.iter().find(|s| s["id"] == session))
                .and_then(|s| s["selectedId"].as_str())
                .ok_or_else(|| io::Error::other("Missing conversation"))?
                .to_string();
            let link = Arc::new(Mutex::new((client.clone(), session.clone())));
            let heartbeat = link.clone();
            let events = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(15)).await;
                    let (client, session) = heartbeat.lock().await.clone();
                    let _ = client
                        .command(json!({"op":"heartbeat","session":session}))
                        .await;
                }
            });
            Ok::<_, io::Error>((link, conversation, events))
        })?;
        Ok(Self {
            runtime,
            link,
            conversation,
            model: config.model.clone(),
            events,
            host: config.base_url.clone(),
        })
    }
    pub fn name(&self) -> &str {
        &self.model
    }
    pub fn select(&mut self, name: String) {
        self.model = name;
    }
    pub fn load(&self, name: &str) -> ModelOperation {
        self.manage(name, true)
    }
    pub fn unload(&self, name: &str) -> ModelOperation {
        self.manage(name, false)
    }
    fn manage(&self, name: &str, load: bool) -> ModelOperation {
        let link = self.link.clone();
        let host = self.host.clone();
        let selected = self.model.clone();
        let conversation = self.conversation.clone();
        let name = name.to_string();
        let (tx, result) = mpsc::channel();
        let name_for_cancel = name.clone();
        let task = self.runtime.spawn(async move {
            let outcome = async {
                let (client, session) = connected(&link, &selected, &host, &conversation).await?;
                client.command(
                    json!({"op":if load{"prepare"}else{"release"},"session":session,"model":name}),
                )
                .await
                .and_then(|v| {
                    if load {
                        let s = v["sessions"]
                            .as_array()
                            .and_then(|a| a.iter().find(|s| s["id"] == session))
                            .ok_or("Session closed")?;
                        if s["activeModel"].as_str() != Some(&name) {
                            return Err(s["error"]
                                .as_str()
                                .unwrap_or("Model could not be loaded")
                                .into());
                        }
                    }
                    Ok(())
                })
            }
            .await;
            let _ = tx.send(outcome);
        });
        ModelOperation {
            result,
            task,
            link: self.link.clone(),
            model: name_for_cancel,
            load,
            runtime: self.runtime.handle().clone(),
        }
    }
    pub fn start(&self, input: String) -> Generation {
        let link = self.link.clone();
        let conversation = self.conversation.clone();
        let (tx, updates) = mpsc::channel();
        let task_link = link.clone();
        let model = self.model.clone();
        let host = self.host.clone();
        let task_conversation = conversation.clone();
        let request_id = lorca::types::id();
        let request = request_id.clone();
        let task = self.runtime.spawn(async move {
            let result = async {
                let (client, session) =
                    connected(&task_link, &model, &host, &task_conversation).await?;
                terminal_reply(&client, &session, &task_conversation, &request, input, &tx).await
            }
            .await;
            if let Err(error) = result {
                let _ = tx.send(Update::Error(error));
            }
        });
        Generation {
            updates,
            task,
            link,
            conversation,
            runtime: self.runtime.handle().clone(),
            request_id,
        }
    }
}
impl Drop for Ollama {
    fn drop(&mut self) {
        self.events.abort();
        self.runtime.block_on(async {
            let (client, session) = self.link.lock().await.clone();
            let _ = client
                .command(json!({"op":"close","session":session,"unload":false}))
                .await;
        });
    }
}
async fn connected(
    link: &Link,
    model: &str,
    host: &str,
    conversation: &str,
) -> Result<(Client, String), String> {
    let mut current = link.lock().await;
    if current
        .0
        .command(json!({"op":"heartbeat","session":current.1}))
        .await
        .is_err()
    {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let client = Client::connect(&executable)
            .await
            .map_err(|e| e.to_string())?;
        let session = client.attach(model, host, false).await?;
        client
            .command(json!({"op":"open","session":session,"conversationId":conversation}))
            .await?;
        *current = (client, session);
    }
    Ok(current.clone())
}

struct StreamGuard<T>(JoinHandle<T>);
impl<T> Drop for StreamGuard<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn terminal_reply(
    client: &Client,
    session: &str,
    conversation: &str,
    request: &str,
    input: String,
    output: &mpsc::Sender<Update>,
) -> Result<(), String> {
    client.command(json!({"op":"send","session":session,"conversationId":conversation,"requestId":request,"text":input})).await?;
    let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
    let c = client.clone();
    let s = session.to_string();
    let stream = StreamGuard(tokio::spawn(async move { c.events(&s, send).await }));
    let mut previous = String::new();
    while let Some(state) = receive.recv().await {
        if let Some(chat) = state["conversations"]
            .as_array()
            .and_then(|chats| chats.iter().find(|c| c["id"] == conversation))
            && let Some(last) = chat["messages"]
                .as_array()
                .and_then(|messages| messages.last())
                .filter(|m| m["role"] == "assistant")
        {
            let text = last["content"].as_str().unwrap_or("");
            if let Some(delta) = text.strip_prefix(&previous).filter(|s| !s.is_empty()) {
                output
                    .send(Update::Text(delta.into()))
                    .map_err(|_| "Response cancelled")?;
                previous = text.into();
            }
        }
        if let Some(turn) = state["turns"]
            .as_array()
            .and_then(|turns| turns.iter().find(|t| t["id"] == request))
            && turn["running"] == false
        {
            if let Some(error) = turn["error"].as_str() {
                return Err(error.into());
            }
            let usage = serde_json::from_value(turn["usage"].clone()).unwrap_or_default();
            let _ = output.send(Update::Done(usage));
            drop(stream);
            return Ok(());
        }
    }
    Err(
        "Lost connection to lorca’s backend. The partial reply is saved; relaunch to reconnect."
            .into(),
    )
}
