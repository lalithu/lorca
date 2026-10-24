use crate::agent::{FutureResult, Inference, Result, ToolCall, Turn};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::time::Duration;

pub struct OllamaAgent {
    pub client: reqwest::Client,
    pub host: String,
    pub model: String,
}
impl Inference for OllamaAgent {
    fn generate<'a>(&'a self, messages: &'a [Value], tools: &'a [Value]) -> FutureResult<'a, Turn> {
        Box::pin(async move {
            let body = json!({"model":self.model,"messages":messages,"tools":tools,"stream":false,"think":false,"keep_alive":-1,
                "options":{"num_ctx":16384,"num_predict":2048,"temperature":0.1}});
            let response = self
                .client
                .post(format!("{}/api/chat", self.host))
                .json(&body)
                .timeout(Duration::from_secs(180))
                .send()
                .await
                .map_err(|e| format!("Ollama: {e}"))?;
            let status = response.status();
            let mut stream = response.bytes_stream();
            let mut bytes = vec![];
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| e.to_string())?;
                if bytes.len() + chunk.len() > 256 * 1024 {
                    return Err("Ollama response exceeded 256 KiB".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            let v: Value =
                serde_json::from_slice(&bytes).map_err(|_| "Invalid Ollama agent response")?;
            if !status.is_success() || v["error"].is_string() {
                return Err(format!(
                    "Ollama: {}",
                    v["error"].as_str().unwrap_or(status.as_str())
                ));
            }
            if v["done"] != true {
                return Err("Ollama ended without a completion acknowledgement".into());
            }
            parse_turn(v["message"].clone())
        })
    }
}
pub fn parse_turn(message: Value) -> Result<Turn> {
    if message["role"] != "assistant" {
        return Err("Expected an assistant message".into());
    }
    let content = message["content"].as_str().unwrap_or("").to_string();
    let mut calls = vec![];
    let mut ids = std::collections::HashSet::new();
    if let Some(raw) = message.get("tool_calls") {
        let raw = raw.as_array().ok_or("tool_calls must be an array")?;
        if raw.len() > 8 {
            return Err("Too many tool calls".into());
        }
        for call in raw {
            let name = call["function"]["name"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 100)
                .ok_or("Missing tool name")?
                .to_string();
            let arguments = match &call["function"]["arguments"] {
                Value::Object(_) => call["function"]["arguments"].clone(),
                Value::String(s) => {
                    serde_json::from_str(s).map_err(|_| "Invalid tool JSON arguments")?
                }
                _ => return Err("Tool arguments must be a JSON object".into()),
            };
            if !arguments.is_object() {
                return Err("Tool arguments must be a JSON object".into());
            }
            let id = call["id"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(crate::types::id);
            if id.is_empty()
                || id.len() > 200
                || id.chars().any(char::is_control)
                || !ids.insert(id.clone())
            {
                return Err("Invalid or duplicate tool call ID".into());
            }
            calls.push(ToolCall {
                id,
                name,
                arguments,
            });
        }
    }
    // Retain the structured assistant turn, without hidden thinking. Normalize optional
    // IDs so every result is correlated even with Ollama versions that omit call IDs.
    let mut wire = json!({"role":"assistant","content":content});
    if !calls.is_empty() {
        wire["tool_calls"]=json!(calls.iter().map(|c|json!({"id":c.id,"type":"function","function":{"name":c.name,"arguments":c.arguments}})).collect::<Vec<_>>());
    }
    Ok(Turn {
        content,
        calls,
        message: wire,
    })
}
