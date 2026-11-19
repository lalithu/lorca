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
        "properties":{"kind":{"const":"table"},"title":string,
            "payload":{"type":"object","additionalProperties":false,"properties":{
                "headers":{"type":"array","minItems":1,"maxItems":6,"items":string},
                "rows":{"type":"array","minItems":1,"maxItems":30,"items":{"type":"array","minItems":1,"maxItems":6,"items":string}}},
                "required":["headers","rows"]}},
        "required":["kind","title","payload"]});
    let text = json!({"type":"object","additionalProperties":false,
        "properties":{"kind":{"enum":["structure","priorities","breakdown","research","checklist"]},"title":string,
            "payload":{"type":"object","additionalProperties":false,"properties":{"text":string},"required":["text"]}},
        "required":["kind","title","payload"]});
    json!({"type":"object","additionalProperties":false,"properties":{
        "suggestions":{"type":"array","maxItems":1,"items":{"anyOf":[table,text]}}},"required":["suggestions"]})
}

pub enum ResponseMode {
    Notes,
    Research,
    Image,
}

pub async fn generate(
    client: &reqwest::Client,
    engine: Engine,
    host: &str,
    model: &str,
    messages: &[Value],
    budget: GenerationBudget,
    mode: ResponseMode,
) -> Result<String, String> {
    let mut schema = schema();
    if matches!(mode, ResponseMode::Research) {
        // A sourced answer is one annotation, never a replacement table that
        // cannot carry its references, or a second search request.
        let mut annotation = schema["properties"]["suggestions"]["items"]["anyOf"][1].clone();
        annotation["properties"]["kind"] = json!({"enum":["breakdown","checklist"]});
        annotation["properties"]["payload"]["properties"]["text"] =
            json!({"type":"string","maxLength":2000});
        schema["properties"]["suggestions"]["items"] = annotation;
    }
    if matches!(mode, ResponseMode::Image) {
        let string = json!({"type":"string"});
        let optional_string = json!({"anyOf":[{"type":"string"},{"type":"null"}]});
        schema["properties"]["suggestions"]["minItems"] = json!(1);
        schema["properties"]["suggestions"]["items"] = json!({"type":"object","additionalProperties":false,
            "properties":{"kind":{"const":"caption"},"title":string,"payload":{"type":"object","additionalProperties":false,
                "properties":{"text":{"type":"string","maxLength":2000},"event":{"anyOf":[{"type":"null"},
                    {"type":"object","additionalProperties":false,"properties":{"title":string,
                        "dateIndex":{"anyOf":[{"type":"integer","minimum":0,"maximum":7},{"type":"null"}]},"location":optional_string},
                        "required":["title","dateIndex","location"]}]}},"required":["text","event"]}},
            "required":["kind","title","payload"]});
    }
    budget.check(
        crate::context::estimate(&json!(messages)) + crate::context::estimate(&schema) + 128,
    )?;
    let wire_messages: Vec<Value> = if matches!(engine, Engine::LlamaCpp) {
        messages.iter().map(|message| {
            let mut message = message.clone();
            if let Some(images) = message["images"].as_array().filter(|a| !a.is_empty()) {
                let mut content = vec![json!({"type":"text","text":message["content"].as_str().unwrap_or_default()})];
                content.extend(images.iter().filter_map(Value::as_str).map(|image| json!({"type":"image_url","image_url":{"url":format!("data:image/jpeg;base64,{image}")}})));
                message["content"] = json!(content);
                message.as_object_mut().unwrap().remove("images");
            }
            message
        }).collect()
    } else {
        messages.to_vec()
    };
    let messages = &wire_messages;
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
