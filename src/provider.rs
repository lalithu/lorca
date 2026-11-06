pub mod agent;
pub mod llama_cpp;
use crate::types::Usage;
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender as Sender;
pub enum Update {
    Text(String),
    Done(Usage),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    #[default]
    Ollama,
    LlamaCpp,
}

impl Engine {
    pub fn default_host(self) -> &'static str {
        match self {
            Self::Ollama => "http://127.0.0.1:11434",
            Self::LlamaCpp => "http://127.0.0.1:9931",
        }
    }
}

pub async fn detect_local_engine(client: &reqwest::Client) -> Engine {
    let healthy = client
        .get("http://127.0.0.1:9931/health")
        .timeout(Duration::from_millis(500))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success());
    if healthy
        && model_list(client, Engine::LlamaCpp, Engine::LlamaCpp.default_host())
            .await
            .is_ok_and(|(models, _)| !models.is_empty())
    {
        Engine::LlamaCpp
    } else {
        Engine::Ollama
    }
}

pub async fn manage(
    client: &reqwest::Client,
    engine: Engine,
    base_url: &str,
    name: &str,
    load: bool,
) -> Result<(), String> {
    match engine {
        Engine::Ollama => manage_model(client, base_url, name, load).await,
        Engine::LlamaCpp => {
            if load {
                // Reuse a model already loaded by the Llama app; an explicit load
                // can restart its allocation and discard the warm prompt cache.
                let status: Value = client
                    .get(format!("{base_url}/models"))
                    .timeout(Duration::from_secs(3))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?
                    .error_for_status()
                    .map_err(|e| e.to_string())?
                    .json()
                    .await
                    .map_err(|e| e.to_string())?;
                if status["data"].as_array().is_some_and(|models| {
                    models.iter().any(|m| {
                        m["id"].as_str().is_some_and(|id| same_model(id, name))
                            && m["status"]["value"] == "loaded"
                    })
                }) {
                    return Ok(());
                }
            }
            let action = if load { "load" } else { "unload" };
            let response = client
                .post(format!("{base_url}/models/{action}"))
                .json(&json!({"model":name}))
                .timeout(Duration::from_secs(if load { 120 } else { 10 }))
                .send()
                .await
                .map_err(|e| format!("Couldn’t {action} {name} through llama.cpp: {e}"))?;
            let status = response.status();
            let body: Value = response
                .json()
                .await
                .map_err(|_| format!("Invalid llama.cpp {action} response"))?;
            if !status.is_success() || body["success"] != true {
                return Err(format!(
                    "llama.cpp {action} failed: {}",
                    body["error"]
                        .as_str()
                        .unwrap_or("server did not confirm the operation")
                ));
            }
            if !load {
                let models: Value = client
                    .get(format!("{base_url}/models"))
                    .timeout(Duration::from_secs(3))
                    .send()
                    .await
                    .map_err(|_| "llama.cpp did not confirm unloading".to_string())?
                    .error_for_status()
                    .map_err(|_| "llama.cpp did not confirm unloading".to_string())?
                    .json()
                    .await
                    .map_err(|_| "Invalid llama.cpp model status".to_string())?;
                let entries = models["data"]
                    .as_array()
                    .ok_or("Invalid llama.cpp model status")?;
                if entries.iter().any(|m| m["id"].as_str().is_none()) {
                    return Err("Invalid llama.cpp model status".into());
                }
                if entries.iter().any(|m| {
                    same_model(m["id"].as_str().unwrap_or(""), name)
                        && m["status"]["value"] == "loaded"
                }) {
                    return Err("llama.cpp still reports the model loaded".into());
                }
            }
            Ok(())
        }
    }
}

pub async fn model_list(
    client: &reqwest::Client,
    engine: Engine,
    base_url: &str,
) -> Result<(Vec<String>, Option<String>), String> {
    match engine {
        Engine::Ollama => {
            let value: Value = client
                .get(format!("{base_url}/api/tags"))
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .map_err(|_| {
                    "Ollama isn’t available. Start it with ‘ollama serve’, then reconnect."
                        .to_string()
                })?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .json()
                .await
                .map_err(|_| "Invalid Ollama model list".to_string())?;
            let mut models = value["models"]
                .as_array()
                .ok_or("Invalid Ollama model list")?
                .iter()
                .filter_map(|m| m["name"].as_str().map(String::from))
                .collect::<Vec<_>>();
            models.sort();
            Ok((models, None))
        }
        Engine::LlamaCpp => {
            let value: Value = client
                .get(format!("{base_url}/models"))
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .map_err(|_| "llama.cpp isn’t available. Open the Llama app and keep its local server running.".to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .json()
                .await
                .map_err(|_| "Invalid llama.cpp model list".to_string())?;
            let entries = value["data"]
                .as_array()
                .ok_or("Invalid llama.cpp model list")?;
            let mut models = Vec::new();
            let mut loaded = None;
            for entry in entries {
                let Some(id) = entry["id"].as_str().filter(|id| !id.is_empty()) else {
                    continue;
                };
                models.push(id.to_owned());
                if entry["status"]["value"] == "loaded" && loaded.is_none() {
                    loaded = Some(id.to_owned());
                }
            }
            models.sort();
            Ok((models, loaded))
        }
    }
}
impl Usage {
    pub(crate) fn from_response(value: &Value) -> Self {
        let generated = value["eval_count"].as_u64();
        let duration = value["eval_duration"].as_u64().filter(|v| *v > 0);
        Self {
            context_used: value["prompt_eval_count"]
                .as_u64()
                .zip(generated)
                .map(|(p, r)| p.saturating_add(r)),
            context_limit: None,
            prompt_tokens: value["prompt_eval_count"].as_u64(),
            generated_tokens: generated,
            generation_duration_ns: duration,
            prompt_duration_ns: value["prompt_eval_duration"].as_u64().filter(|n| *n > 0),
            prompt_tokens_per_second: value["prompt_eval_count"]
                .as_u64()
                .zip(value["prompt_eval_duration"].as_u64().filter(|n| *n > 0))
                .map(|(n, ns)| n as f64 * 1e9 / ns as f64),
            tokens_per_second: generated
                .zip(duration)
                .map(|(t, n)| t as f64 * 1e9 / n as f64),
        }
    }
}
pub fn chat_body(model: &str, messages: Vec<Value>) -> Value {
    json!({ "model": model, "messages": messages, "stream": true, "think": false, "keep_alive": -1 })
}

pub fn chat_body_budgeted(
    model: &str,
    messages: Vec<Value>,
    budget: crate::context::GenerationBudget,
) -> Value {
    let mut body = chat_body(model, messages);
    body["options"] = json!({"num_ctx":budget.context_limit,"num_predict":budget.output_limit});
    body
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

pub async fn manage_model(
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

pub async fn stream_reply(
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
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        assert!(
            parse_line(
                r#"{"message":{"content":"orca 🐋"},"done":true}"#.as_bytes(),
                &sender
            )
            .unwrap()
            .is_some()
        );
        assert!(matches!(receiver.try_recv().unwrap(), Update::Text(text) if text == "orca 🐋"));
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
        let (sender, _) = tokio::sync::mpsc::unbounded_channel();
        assert_eq!(
            parse_line(br#"{"error":"model missing"}"#, &sender).unwrap_err(),
            "model missing"
        );
        assert!(parse_line(b"not json", &sender).is_err());
    }
}
