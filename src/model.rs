use std::{
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::{runtime::Runtime, task::JoinHandle};

use crate::{chat::Message, config::Config};

pub enum Update {
    Text(String),
    Done(Usage),
    Error(String),
}

#[derive(Debug, Default)]
pub struct Usage {
    pub context_used: Option<u64>,
    pub context_limit: Option<u64>,
    pub tokens_per_second: Option<f64>,
}

impl Usage {
    fn from_response(value: &Value) -> Self {
        let generated = value["eval_count"].as_u64();
        let duration = value["eval_duration"]
            .as_u64()
            .filter(|duration| *duration > 0);
        Self {
            context_used: value["prompt_eval_count"]
                .as_u64()
                .zip(generated)
                .map(|(prompt, reply)| prompt.saturating_add(reply)),
            context_limit: None,
            tokens_per_second: generated
                .zip(duration)
                .map(|(tokens, nanos)| tokens as f64 * 1_000_000_000.0 / nanos as f64),
        }
    }
}

pub struct Ollama {
    runtime: Runtime,
    client: reqwest::Client,
    base_url: String,
    model: String,
}

pub struct Generation {
    pub updates: Receiver<Update>,
    task: JoinHandle<()>,
}

pub struct ModelOperation {
    pub result: Receiver<Result<(), String>>,
    task: JoinHandle<()>,
}

impl Drop for ModelOperation {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Drop for Generation {
    fn drop(&mut self) {
        // Dropping the HTTP stream stops generation on Ollama as well.
        self.task.abort();
    }
}

impl Ollama {
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
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let name = name.to_string();
        let (sender, result) = mpsc::channel();
        let task = self.runtime.spawn(async move {
            let outcome = manage_model(&client, &base_url, &name, load).await;
            let _ = sender.send(outcome);
        });
        ModelOperation { result, task }
    }

    pub fn new(config: &Config) -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()?;
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(std::io::Error::other)?;
        Ok(Self {
            runtime,
            client,
            base_url: config.base_url.clone(),
            model: config.model.clone(),
        })
    }

    pub fn start(&self, messages: &[Message]) -> Generation {
        let messages: Vec<Value> = messages
            .iter()
            .map(|message| {
                json!({
                    "role": message.role, "content": message.content
                })
            })
            .collect();
        let body = chat_body(&self.model, messages);
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let (sender, updates) = mpsc::channel();
        let task = self.runtime.spawn(async move {
            if let Err(error) = stream_reply(client, &base_url, body, &sender).await {
                let _ = sender.send(Update::Error(error));
            }
        });
        Generation { updates, task }
    }
}

fn chat_body(model: &str, messages: Vec<Value>) -> Value {
    json!({ "model": model, "messages": messages, "stream": true, "think": false, "keep_alive": -1 })
}

pub fn same_model(left: &str, right: &str) -> bool {
    fn qualified(name: &str) -> String {
        if name.rsplit('/').next().unwrap_or(name).contains(':') {
            name.to_string()
        } else {
            format!("{name}:latest")
        }
    }
    qualified(left) == qualified(right)
}

async fn manage_model(
    client: &reqwest::Client,
    base_url: &str,
    name: &str,
    load: bool,
) -> Result<(), String> {
    let action = if load { "load" } else { "unload" };
    let response = client
        .post(format!("{base_url}/api/generate"))
        .json(&json!({"model": name, "stream": false, "keep_alive": if load { -1 } else { 0 }}))
        .timeout(Duration::from_secs(if load { 120 } else { 5 }))
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                format!("Ollama {action} timed out; model state is unconfirmed")
            } else {
                format!("Cannot {action} {name} at {base_url}. Check Ollama is running.")
            }
        })?;
    let status = response.status();
    let value: Value = response
        .json()
        .await
        .map_err(|_| format!("Invalid Ollama {action} acknowledgement"))?;
    if !status.is_success() || value["error"].is_string() {
        return Err(format!(
            "Ollama {status}: {}",
            value["error"].as_str().unwrap_or("model request failed")
        ));
    }
    if value["done"].as_bool() != Some(true) {
        return Err(format!("Ollama did not confirm model {action}"));
    }
    if let Some(returned) = value["model"].as_str()
        && !same_model(returned, name)
    {
        return Err(format!(
            "Ollama acknowledged a different model during {action}"
        ));
    }
    if !load && value["done_reason"].as_str() != Some("unload") {
        // Older servers may omit done_reason. A successful /ps response with
        // the model absent is also an explicit server confirmation.
        let response = client
            .get(format!("{base_url}/api/ps"))
            .timeout(Duration::from_secs(1))
            .send()
            .await
            .map_err(|_| "Ollama did not confirm unloading".to_string())?;
        if !response.status().is_success() {
            return Err("Ollama did not confirm unloading".into());
        }
        let running: Value = response
            .json()
            .await
            .map_err(|_| "Invalid Ollama model status".to_string())?;
        let models = running["models"]
            .as_array()
            .ok_or("Invalid Ollama model status")?;
        if models.iter().any(|model| {
            model["name"]
                .as_str()
                .or_else(|| model["model"].as_str())
                .is_none()
        }) {
            return Err("Invalid Ollama model status".into());
        }
        if models.iter().any(|model| {
            same_model(
                model["name"]
                    .as_str()
                    .or_else(|| model["model"].as_str())
                    .unwrap_or(""),
                name,
            )
        }) {
            return Err("Ollama still reports the model loaded".into());
        }
    }
    Ok(())
}

async fn stream_reply(
    client: reqwest::Client,
    base_url: &str,
    body: Value,
    sender: &Sender<Update>,
) -> Result<(), String> {
    let response = client.post(format!("{base_url}/api/chat")).json(&body).send().await
        .map_err(|_| format!("Cannot connect to Ollama at {base_url}. Start it with `ollama serve` and check --host."))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.json::<Value>().await.unwrap_or_default();
        let error = body["error"].as_str().unwrap_or("request failed");
        return Err(format!("Ollama {status}: {error}"));
    }
    let mut stream = response.bytes_stream();
    let mut pending = Vec::new();
    while let Some(chunk) = stream.next().await {
        pending.extend_from_slice(&chunk.map_err(|_| "Ollama connection interrupted".to_string())?);
        while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            if let Some(usage) = parse_line(&line, sender)? {
                finish(&client, base_url, &body, sender, usage).await;
                return Ok(());
            }
        }
    }
    if let Some(usage) = parse_line(&pending, sender)? {
        finish(&client, base_url, &body, sender, usage).await;
        return Ok(());
    }
    Err("Ollama ended the response before it finished. You can try again.".into())
}

async fn finish(
    client: &reqwest::Client,
    base_url: &str,
    body: &Value,
    sender: &Sender<Update>,
    mut usage: Usage,
) {
    // Use the running model's allocated window, not its theoretical maximum.
    // Metadata is best effort and must never make a successful reply fail.
    if let Ok(response) = client
        .get(format!("{base_url}/api/ps"))
        .timeout(Duration::from_secs(1))
        .send()
        .await
        && let Ok(value) = response.json::<Value>().await
        && let Some(models) = value["models"].as_array()
    {
        let requested = body["model"].as_str().unwrap_or_default();
        usage.context_limit = models
            .iter()
            .find(|model| {
                let name = model["name"]
                    .as_str()
                    .or_else(|| model["model"].as_str())
                    .unwrap_or_default();
                name == requested || name == format!("{requested}:latest")
            })
            .and_then(|model| model["context_length"].as_u64())
            .filter(|limit| *limit > 0);
    }
    if let (Some(used), Some(limit)) = (usage.context_used, usage.context_limit) {
        usage.context_used = Some(used.min(limit));
    }
    let _ = sender.send(Update::Done(usage));
}

fn parse_line(line: &[u8], sender: &Sender<Update>) -> Result<Option<Usage>, String> {
    if line.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    let value: Value =
        serde_json::from_slice(line).map_err(|_| "Invalid response from Ollama".to_string())?;
    if let Some(error) = value["error"].as_str() {
        return Err(error.to_string());
    }
    if let Some(text) = value["message"]["content"]
        .as_str()
        .filter(|text| !text.is_empty())
    {
        sender
            .send(Update::Text(text.to_string()))
            .map_err(|_| "Response cancelled".to_string())?;
    }
    if value["done"].as_bool() == Some(true) {
        return Ok(Some(Usage::from_response(&value)));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_server(replies: Vec<(u16, Value)>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let task = std::thread::spawn(move || {
            let mut bodies = Vec::new();
            for (status, reply) in replies {
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    if let Ok((stream, _)) = listener.accept() {
                        break stream;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "No lifecycle request received"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let (header_end, length) = loop {
                    let mut buffer = [0; 1024];
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let header = std::str::from_utf8(&bytes[..end]).unwrap();
                        let length = header
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        break (end + 4, length);
                    }
                };
                while bytes.len() < header_end + length {
                    let mut buffer = [0; 1024];
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                }
                bodies.push(
                    serde_json::from_slice(&bytes[header_end..header_end + length])
                        .unwrap_or(Value::Null),
                );
                let reply = reply.to_string();
                write!(stream, "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
            }
            bodies
        });
        (address, task)
    }

    #[test]
    fn lifecycle_pins_and_requires_server_confirmation_to_unload() {
        let (base_url, server) = mock_server(vec![
            (
                200,
                json!({"model": "orca", "done": true, "done_reason": "load"}),
            ),
            (200, json!({"model": "orca", "done": true})),
            (200, json!({"models": [{"name": "orca:latest"}]})),
            (
                200,
                json!({"model": "orca", "done": true, "done_reason": "unload"}),
            ),
        ]);
        let model = Ollama::new(&Config {
            model: "orca".into(),
            base_url,
        })
        .unwrap();
        assert!(
            model
                .load("orca")
                .result
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .is_ok()
        );
        assert!(
            model
                .unload("orca")
                .result
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .is_err()
        );
        assert!(
            model
                .unload("orca")
                .result
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .is_ok()
        );
        let bodies = server.join().unwrap();
        assert_eq!(bodies[0]["keep_alive"], -1);
        assert_eq!(bodies[0]["stream"], false);
        assert!(bodies[0].get("prompt").is_none());
        assert_eq!(bodies[1]["keep_alive"], 0);
        assert_eq!(bodies[3]["keep_alive"], 0);
    }

    #[test]
    fn lifecycle_reports_server_load_failure() {
        let (base_url, server) = mock_server(vec![(500, json!({"error": "not enough memory"}))]);
        let model = Ollama::new(&Config {
            model: "orca".into(),
            base_url,
        })
        .unwrap();
        let error = model
            .load("orca")
            .result
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap_err();
        assert!(error.contains("not enough memory"));
        server.join().unwrap();
    }

    #[test]
    fn every_chat_keeps_the_selected_model_loaded() {
        let body = chat_body("orca", vec![json!({"role": "user", "content": "hello"})]);
        assert_eq!(body["keep_alive"], -1);
        assert_eq!(body["model"], "orca");
        assert!(same_model("orca", "orca:latest"));
        assert!(!same_model("orca:9b", "orca:latest"));
        assert!(same_model(
            "localhost:5000/orca",
            "localhost:5000/orca:latest"
        ));
    }

    #[test]
    fn final_chunk_keeps_text_before_done() {
        let (sender, receiver) = mpsc::channel();
        assert!(
            parse_line(
                r#"{"message":{"content":"orca 🐋"},"done":true}"#.as_bytes(),
                &sender
            )
            .unwrap()
            .is_some()
        );
        assert!(matches!(receiver.recv().unwrap(), Update::Text(text) if text == "orca 🐋"));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn usage_counts_prompt_and_reply_and_uses_generation_time() {
        let usage = Usage::from_response(&json!({
            "prompt_eval_count": 120, "eval_count": 80, "eval_duration": 2_000_000_000_u64
        }));
        assert_eq!(usage.context_used, Some(200));
        assert_eq!(usage.tokens_per_second, Some(40.0));
        let missing = Usage::from_response(&json!({"eval_duration": 0}));
        assert_eq!(missing.context_used, None);
        assert_eq!(missing.tokens_per_second, None);
    }

    #[test]
    fn stream_errors_are_reported() {
        let (sender, _) = mpsc::channel();
        assert_eq!(
            parse_line(br#"{"error":"model missing"}"#, &sender).unwrap_err(),
            "model missing"
        );
        assert!(parse_line(b"not json", &sender).is_err());
    }
}
