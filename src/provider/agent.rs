use crate::agent::{FutureResult, Inference, ModelUpdate, Result, ToolCall, Turn};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::time::Duration;

pub struct OllamaAgent {
    pub client: reqwest::Client,
    pub host: String,
    pub model: String,
}
impl OllamaAgent {
    async fn allocated(&self) -> Option<u64> {
        let v: Value = self
            .client
            .get(format!("{}/api/ps", self.host))
            .timeout(Duration::from_secs(1))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        v["models"].as_array()?.iter().find(|m| {
            m["name"]
                .as_str()
                .or_else(|| m["model"].as_str())
                .is_some_and(|n| super::same_model(n, &self.model))
        })?["context_length"]
            .as_u64()
            .filter(|n| *n > 0)
    }
}
impl Inference for OllamaAgent {
    fn capacity(&self) -> FutureResult<'_, Option<u64>> {
        Box::pin(async { Ok(self.allocated().await) })
    }
    fn generate<'a>(&'a self, messages: &'a [Value], tools: &'a [Value]) -> FutureResult<'a, Turn> {
        self.generate_stream(messages, tools, &|_| {})
    }
    fn generate_stream<'a>(
        &'a self,
        messages: &'a [Value],
        tools: &'a [Value],
        updates: &'a (dyn Fn(ModelUpdate) + Send + Sync),
    ) -> FutureResult<'a, Turn> {
        Box::pin(async move {
            if messages
                .iter()
                .any(|m| m["images"].as_array().is_some_and(|a| !a.is_empty()))
            {
                let response = self
                    .client
                    .post(format!("{}/api/show", self.host))
                    .json(&json!({"model":self.model}))
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                if response.status().is_success() {
                    let info: Value = response.json().await.map_err(|e| e.to_string())?;
                    if info["capabilities"]
                        .as_array()
                        .is_some_and(|a| !a.iter().any(|v| v == "vision"))
                    {
                        return Err(format!(
                            "{} doesn't support images. Choose a vision-capable model. Your images are saved.",
                            self.model
                        ));
                    }
                }
            }
            let capacity = self
                .allocated()
                .await
                .unwrap_or(crate::context::DEFAULT_CAPACITY);
            let mut body = json!({"model":self.model,"messages":messages,"stream":true,"think":false,"keep_alive":-1,
                "options":{"num_ctx":capacity,"num_predict":(capacity/4).min(2048),"temperature":0.1}});
            if !tools.is_empty() {
                body["tools"] = json!(tools);
            }
            let response = self
                .client
                .post(format!("{}/api/chat", self.host))
                .json(&body)
                .timeout(Duration::from_secs(180))
                .send()
                .await
                .map_err(|e| format!("Ollama: {e}"))?;
            if !response.status().is_success() {
                let status = response.status();
                let v: Value = response.json().await.unwrap_or_default();
                return Err(format!(
                    "Ollama {status}: {}",
                    v["error"].as_str().unwrap_or("request failed")
                ));
            }
            let mut stream = response.bytes_stream();
            let mut pending = Vec::new();
            let mut assembly = StreamTurn::default();
            let mut done = None;
            while let Some(chunk) = stream.next().await {
                pending.extend_from_slice(
                    &chunk.map_err(|e| format!("Ollama connection interrupted: {e}"))?,
                );
                if pending.len() > 256 * 1024 {
                    return Err("Ollama stream frame exceeded 256 KiB".into());
                }
                while let Some(end) = pending.iter().position(|b| *b == b'\n') {
                    let line: Vec<_> = pending.drain(..=end).collect();
                    if line.iter().all(u8::is_ascii_whitespace) {
                        continue;
                    }
                    let v: Value =
                        serde_json::from_slice(&line).map_err(|_| "Invalid Ollama stream frame")?;
                    let text = assembly.push(&v)?;
                    if !text.is_empty() {
                        updates(ModelUpdate::Text(text));
                    }
                    if v["done"] == true {
                        done = Some(v);
                        break;
                    }
                }
                if done.is_some() {
                    break;
                }
            }
            if done.is_none() && !pending.iter().all(u8::is_ascii_whitespace) {
                let v: Value = serde_json::from_slice(&pending)
                    .map_err(|_| "Invalid final Ollama stream frame")?;
                let text = assembly.push(&v)?;
                if !text.is_empty() {
                    updates(ModelUpdate::Text(text));
                }
                if v["done"] == true {
                    done = Some(v);
                }
            }
            // Tools are never executed until the complete turn is acknowledged.
            let done=done.ok_or("Ollama ended without a completion acknowledgement; partial output was saved, actions were not replayed")?;
            let mut usage = crate::types::Usage::from_response(&done);
            usage.context_limit = self.allocated().await;
            updates(ModelUpdate::Usage(usage));
            parse_turn(assembly.message()?)
        })
    }
}

/// Ollama NDJSON can split anywhere in UTF-8 or JSON. Frames are decoded only
/// once complete; native call indices correlate name/argument fragments.
#[derive(Default)]
pub struct StreamTurn {
    content: String,
    calls: std::collections::BTreeMap<usize, Value>,
}
impl StreamTurn {
    pub fn push(&mut self, v: &Value) -> Result<String> {
        if let Some(e) = v["error"].as_str() {
            return Err(format!("Ollama: {e}"));
        }
        let text = v["message"]["content"].as_str().unwrap_or("");
        self.content.push_str(text);
        if self.content.len() > 128 * 1024 {
            return Err("Assistant response exceeded 128 KiB".into());
        }
        if let Some(calls) = v["message"]["tool_calls"].as_array() {
            let base = self.calls.len();
            for (position, call) in calls.iter().enumerate() {
                let index = call["index"]
                    .as_u64()
                    .or_else(|| call["function"]["index"].as_u64())
                    .map(|n| n as usize)
                    .unwrap_or_else(|| {
                        call["id"]
                            .as_str()
                            .and_then(|id| {
                                self.calls
                                    .iter()
                                    .find(|(_, v)| v["id"] == id)
                                    .map(|(i, _)| *i)
                            })
                            .unwrap_or(base + position)
                    });
                if index >= 8 {
                    return Err("Too many streamed tool calls".into());
                }
                let entry = self
                    .calls
                    .entry(index)
                    .or_insert_with(|| json!({"function":{"name":"","arguments":null}}));
                if let Some(id) = call["id"].as_str() {
                    entry["id"] = json!(id);
                }
                let name = call["function"]["name"].as_str().unwrap_or("");
                if !name.is_empty() {
                    let old = entry["function"]["name"].as_str().unwrap_or("");
                    entry["function"]["name"] = json!(if old == name {
                        old.to_string()
                    } else {
                        format!("{old}{name}")
                    });
                }
                let incoming = &call["function"]["arguments"];
                if incoming.is_object() {
                    if entry["function"]["arguments"].is_object() {
                        for (k, v) in incoming.as_object().unwrap() {
                            entry["function"]["arguments"][k] = v.clone();
                        }
                    } else {
                        entry["function"]["arguments"] = incoming.clone();
                    }
                } else if let Some(fragment) = incoming.as_str() {
                    let old = entry["function"]["arguments"].as_str().unwrap_or("");
                    entry["function"]["arguments"] = json!(format!("{old}{fragment}"));
                }
                if entry.to_string().len() > 64 * 1024 {
                    return Err("Streamed tool arguments exceed 64 KiB".into());
                }
            }
        }
        Ok(text.into())
    }
    pub fn message(self) -> Result<Value> {
        let mut message = json!({"role":"assistant","content":self.content});
        if !self.calls.is_empty() {
            message["tool_calls"] = json!(self.calls.into_values().collect::<Vec<_>>());
        }
        Ok(message)
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
