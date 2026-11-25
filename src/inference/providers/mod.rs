pub mod llama_cpp;
pub mod ollama;
use crate::{inference::*, types::*};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::time::Duration;

pub async fn frames(
    response: reqwest::Response,
    sse: bool,
    on: &mut (dyn FnMut(Value) -> Result<()> + Send),
) -> Result<()> {
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Provider HTTP {status}: {}",
            response.text().await.unwrap_or_default()
        ));
    }
    let mut stream = response.bytes_stream();
    let mut pending = Vec::new();
    let mut complete = false;
    loop {
        let next = tokio::time::timeout(Duration::from_secs(180), stream.next())
            .await
            .map_err(|_| "Model stream inactive for 180 seconds")?;
        let Some(bytes) = next else { break };
        pending.extend_from_slice(&bytes.map_err(|e| e.to_string())?);
        if pending.len() > 4 * 1024 * 1024 {
            return Err("Provider frame exceeds 4 MiB".into());
        }
        while let Some(end) = pending.iter().position(|b| *b == b'\n') {
            let line: Vec<_> = pending.drain(..=end).collect();
            if frame(&line, sse, on)? {
                complete = true;
                if !sse || std::str::from_utf8(&line).unwrap_or("").trim() == "data: [DONE]" {
                    return Ok(());
                }
            }
        }
    }
    if !pending.is_empty() && frame(&pending, sse, on)? {
        complete = true;
    }
    if !complete {
        return Err("Provider connection ended before a completion acknowledgement. Partial work is saved; actions were not replayed.".into());
    }
    Ok(())
}
fn frame(
    bytes: &[u8],
    sse: bool,
    on: &mut (dyn FnMut(Value) -> Result<()> + Send),
) -> Result<bool> {
    let line = std::str::from_utf8(bytes)
        .map_err(|e| e.to_string())?
        .trim();
    let data = if sse {
        let Some(s) = line.strip_prefix("data:") else {
            return Ok(false);
        };
        s.trim()
    } else {
        line
    };
    if data.is_empty() {
        return Ok(false);
    }
    if data == "[DONE]" {
        return Ok(true);
    }
    let v: Value = serde_json::from_str(data).map_err(|e| e.to_string())?;
    if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
        return Err(format!("Provider: {e}"));
    }
    let done = v["done"] == true || v["choices"][0]["finish_reason"].is_string();
    on(v)?;
    Ok(done)
}
pub fn calls(message: &Value) -> Result<Vec<Call>> {
    let mut out = vec![];
    for call in message["tool_calls"].as_array().into_iter().flatten() {
        let name = call["function"]["name"]
            .as_str()
            .ok_or("Tool name is missing")?;
        let raw = &call["function"]["arguments"];
        let args = if let Some(s) = raw.as_str() {
            serde_json::from_str(s)
                .map_err(|e| format!("Incomplete tool arguments for {name}: {e}"))?
        } else {
            raw.clone()
        };
        if !args.is_object() {
            return Err(format!("Arguments for {name} must be an object"));
        }
        out.push(Call {
            id: call["id"].as_str().map(str::to_owned).unwrap_or_else(id),
            name: name.into(),
            arguments: args,
        });
    }
    Ok(out)
}
pub fn message(text: String, calls: &[Call]) -> Value {
    let mut m = json!({"role":"assistant","content":text});
    if !calls.is_empty() {
        m["tool_calls"]=json!(calls.iter().map(|c|json!({"id":c.id,"type":"function","function":{"name":c.name,"arguments":c.arguments}})).collect::<Vec<_>>());
    }
    m
}
pub async fn catalog(client: &reqwest::Client, route: ModelRoute) -> Result<Vec<ModelChoice>> {
    let url = format!(
        "{}{}",
        route.host,
        if route.provider == Engine::Ollama {
            "/api/tags"
        } else {
            "/v1/models"
        }
    );
    let v: Value = client
        .get(url)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let items = if route.provider == Engine::Ollama {
        &v["models"]
    } else {
        &v["data"]
    };
    Ok(items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let model = m["name"].as_str().or(m["id"].as_str())?;
            Some(ModelChoice {
                route: route.clone(),
                model: model.into(),
                managed: route.provider == Engine::Ollama,
                context_limit: m["meta"]["n_ctx_train"]
                    .as_u64()
                    .or(m["context_length"].as_u64()),
            })
        })
        .collect())
}
pub async fn prepare(model: &Model, load: bool) -> Result<Option<u64>> {
    if model.choice.route.provider == Engine::LlamaCpp {
        if !load {
            return Err("Unload llama.cpp models using its server controls.".into());
        }
        let mut url = reqwest::Url::parse(&format!("{}/props", model.choice.route.host))
            .map_err(|e| e.to_string())?;
        url.query_pairs_mut()
            .append_pair("model", &model.choice.model);
        let r = model
            .client
            .get(url)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let v: Value = r
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        *model.properties.lock().unwrap() = v.clone();
        return Ok(v["default_generation_settings"]["n_ctx"]
            .as_u64()
            .or(v["n_ctx"].as_u64()));
    }
    let mut b = json!({"model":model.choice.model,"stream":false,"keep_alive":if load{json!(-1)}else{json!(0)}});
    if load {
        b["messages"] = json!([]);
        if let Some(n) = model.context {
            b["options"] = json!({"num_ctx":n});
        }
    }
    model
        .client
        .post(format!("{}/api/chat", model.choice.route.host))
        .json(&b)
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    if !load {
        return Ok(None);
    }
    let v: Value = model
        .client
        .get(format!("{}/api/ps", model.choice.route.host))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|m| same_model(m["name"].as_str().unwrap_or(""), &model.choice.model))
        .and_then(|m| m["context_length"].as_u64()))
}
pub fn same_model(a: &str, b: &str) -> bool {
    a == b || a.strip_suffix(":latest").unwrap_or(a) == b.strip_suffix(":latest").unwrap_or(b)
}
