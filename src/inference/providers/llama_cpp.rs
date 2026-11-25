use super::*;
pub async fn generate(
    m: &Model,
    messages: &[Value],
    tools: &[Value],
    output: u64,
    on: &(dyn Fn(Update) + Send + Sync),
) -> Result<Turn> {
    let body = request_body(m, messages, tools, output);
    let response = m
        .client
        .post(format!("{}/v1/chat/completions", m.choice.route.host))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let mut text = String::new();
    let mut partial = std::collections::BTreeMap::<u64, Value>::new();
    let mut length = false;
    frames(response, true, &mut |v| {
        let delta = &v["choices"][0]["delta"];
        if let Some(s) = delta["content"].as_str() {
            text.push_str(s);
            on(Update::Text(s.into()));
        }
        if let Some(s) = delta["reasoning_content"]
            .as_str()
            .or(delta["reasoning"].as_str())
        {
            on(Update::Reasoning(s.into()));
        }
        for c in delta["tool_calls"].as_array().into_iter().flatten() {
            let entry = partial
                .entry(c["index"].as_u64().unwrap_or(0))
                .or_insert_with(
                    || json!({"id":id(),"type":"function","function":{"name":"","arguments":""}}),
                );
            if let Some(s) = c["id"].as_str() {
                entry["id"] = json!(s);
            }
            for key in ["name", "arguments"] {
                if let Some(s) = c["function"][key].as_str() {
                    let old = entry["function"][key].as_str().unwrap_or("");
                    entry["function"][key] = json!(format!("{old}{s}"));
                }
            }
        }
        if v["choices"][0]["finish_reason"] == "length" {
            length = true;
        }
        if let Some(p) = v["usage"]["prompt_tokens"].as_u64() {
            let g = v["usage"]["completion_tokens"].as_u64();
            on(Update::Usage(Usage {
                context_used: g.map(|g| p + g),
                context_limit: m.context,
                prompt_tokens: Some(p),
                generated_tokens: g,
                tokens_per_second: v["timings"]["predicted_per_second"].as_f64(),
                ..Default::default()
            }));
        }
        Ok(())
    })
    .await?;
    if length && !partial.is_empty() {
        return Err(
            "Tool call hit the output limit; no arguments executed. Retry with a smaller call."
                .into(),
        );
    }
    let calls = calls(&json!({"tool_calls":partial.into_values().collect::<Vec<_>>()}))?;
    let message = message(text, &calls);
    Ok(Turn {
        message,
        calls,
        length,
    })
}

pub fn wire_messages(messages: &[Value]) -> Vec<Value> {
    let messages:Vec<_>=messages.iter().map(|v|{
        let mut v=v.clone();if let Some(calls)=v["tool_calls"].as_array_mut(){for c in calls {if !c["function"]["arguments"].is_string(){c["function"]["arguments"]=json!(c["function"]["arguments"].to_string());}}}
        if let Some(images)=v.as_object_mut().and_then(|o|o.remove("images")).and_then(|v|v.as_array().cloned()){
            let mut content=vec![json!({"type":"text","text":v["content"].as_str().unwrap_or("")})];
            for img in images {if let Some(s)=img.as_str(){let mime=if s.starts_with("iVBORw0KGgo"){ "png" }else{ "jpeg" };
                            content.push(json!({"type":"image_url","image_url":{"url":format!("data:image/{mime};base64,{s}")}}));}}
            v["content"]=json!(content);
        }v
    }).collect();
    messages
}

pub fn request_body(m: &Model, messages: &[Value], tools: &[Value], output: u64) -> Value {
    let messages = wire_messages(messages);
    let mut body = json!({"model":m.choice.model,"messages":messages,"tools":tools,"stream":true,"max_tokens":output,"reasoning_effort":if m.effort=="off"{"none"}else if m.effort=="high"{"high"}else{"medium"},"stream_options":{"include_usage":true}});
    // Only send thinking controls the server advertises; the loop has no model-name rules.
    body.as_object_mut().unwrap().remove("reasoning_effort");
    let desired = if m.effort == "off" {
        "none"
    } else if m.effort == "high" {
        "high"
    } else {
        "medium"
    };
    if m.properties.lock().unwrap()["chat_template_caps"]["reasoning_effort_levels"]
        .as_array()
        .is_some_and(|levels| levels.iter().any(|v| v == desired))
    {
        body["reasoning_effort"] = json!(desired);
    }
    body
}
