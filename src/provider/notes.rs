//! Schema-constrained, tool-free inference for document proposals. Model
//! lifecycle and cancellation belong to Core; this module only transports data.
use super::Engine;
use crate::context::GenerationBudget;
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::time::Duration;

pub fn schema() -> Value {
    let string = json!({"type":"string"});
    let table = json!({"type":"object","additionalProperties":false,
        "properties":{"kind":{"const":"table"},"source":string,"title":string,
            "payload":{"type":"object","additionalProperties":false,"properties":{
                "headers":{"type":"array","minItems":1,"maxItems":6,"items":string},
                "rows":{"type":"array","minItems":1,"maxItems":30,"items":{"type":"array","minItems":1,"maxItems":6,"items":string}}},
                "required":["headers","rows"]}},
        "required":["kind","source","title","payload"]});
    let text = json!({"type":"object","additionalProperties":false,
        "properties":{"kind":{"enum":["structure","priorities","breakdown"]},"source":string,"title":string,
            "payload":{"type":"object","additionalProperties":false,"properties":{"text":string},"required":["text"]}},
        "required":["kind","source","title","payload"]});
    json!({"type":"object","additionalProperties":false,"properties":{
        "suggestions":{"type":"array","maxItems":2,"items":{"anyOf":[table,text]}}},"required":["suggestions"]})
}

pub async fn generate(
    client: &reqwest::Client,
    engine: Engine,
    host: &str,
    model: &str,
    messages: &[Value],
    budget: GenerationBudget,
) -> Result<String, String> {
    let schema = schema();
    budget.check(
        crate::context::estimate(&json!(messages)) + crate::context::estimate(&schema) + 128,
    )?;
    let (path, body) = match engine {
        Engine::Ollama => (
            "/api/chat",
            json!({"model":model,"messages":messages,"stream":false,
            "think":false,"keep_alive":-1,"format":schema,
            "options":{"num_ctx":budget.context_limit,"num_predict":budget.output_limit,"temperature":0.1}}),
        ),
        Engine::LlamaCpp => (
            "/v1/chat/completions",
            json!({"model":model,"messages":messages,"stream":false,
            "max_tokens":budget.output_limit,"temperature":0.1,
            "chat_template_kwargs":{"enable_thinking":false},
            "response_format":{"type":"json_schema","json_schema":{"name":"notes_proposals","strict":true,"schema":schema}}}),
        ),
    };
    let response = client
        .post(format!("{host}{path}"))
        .json(&body)
        .timeout(Duration::from_secs(75))
        .send()
        .await
        .map_err(|e| format!("Model following connection: {e}"))?;
    let status = response.status();
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Model following interrupted: {e}"))?;
        if bytes.len() + chunk.len() > 65_536 {
            return Err("Model following response exceeded its limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "Unreadable model-following response")?;
    if !status.is_success() {
        return Err(format!(
            "Model following {status}: {}",
            value["error"]
                .as_str()
                .or_else(|| value["error"]["message"].as_str())
                .unwrap_or("request failed")
        ));
    }
    let (message, reason) = match engine {
        Engine::Ollama => {
            if value["done"] != true {
                return Err("Model following returned an incomplete response".into());
            }
            (&value["message"], value["done_reason"].as_str())
        }
        Engine::LlamaCpp => (
            &value["choices"][0]["message"],
            value["choices"][0]["finish_reason"].as_str(),
        ),
    };
    if reason == Some("length")
        || message["tool_calls"]
            .as_array()
            .is_some_and(|calls| !calls.is_empty())
    {
        return Err("Model following returned incomplete output; your note is unchanged".into());
    }
    message["content"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "Model following returned no proposal data".into())
}
