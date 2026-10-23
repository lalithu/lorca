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

impl Drop for Generation {
    fn drop(&mut self) {
        // Dropping the HTTP stream stops generation on Ollama as well.
        self.task.abort();
    }
}

impl Ollama {
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
        let body =
            json!({ "model": self.model, "messages": messages, "stream": true, "think": false });
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
