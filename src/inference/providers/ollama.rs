use super::*;
pub async fn generate(
    m: &Model,
    messages: &[Value],
    tools: &[Value],
    output: u64,
    on: &(dyn Fn(Update) + Send + Sync),
) -> Result<Turn> {
    let mut options = json!({"num_predict":output});
    if let Some(n) = m.context {
        options["num_ctx"] = json!(n);
    }
    let mut body = json!({"model":m.choice.model,"messages":messages,"tools":tools,"stream":true,"keep_alive":-1,"options":options});
    // Capability metadata controls thinking; no model-name rules.
    if let Ok(r) = m
        .client
        .post(format!("{}/api/show", m.choice.route.host))
        .json(&json!({"model":m.choice.model}))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        && let Ok(info) = r.json::<Value>().await
        && info["capabilities"]
            .as_array()
            .is_some_and(|a| a.iter().any(|c| c == "thinking"))
    {
        body["think"] = json!(m.effort != "off");
    }
    let response = m
        .client
        .post(format!("{}/api/chat", m.choice.route.host))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let mut text = String::new();
    let mut tool_calls = vec![];
    let mut length = false;
    frames(response, false, &mut |v| {
        if let Some(s) = v["message"]["content"].as_str() {
            text.push_str(s);
            on(Update::Text(s.into()));
        }
        if let Some(s) = v["message"]["thinking"].as_str() {
            on(Update::Reasoning(s.into()));
        }
        if let Some(c) = v["message"]["tool_calls"].as_array() {
            tool_calls.extend(c.iter().cloned());
        }
        if v["done"] == true {
            length = v["done_reason"] == "length";
            let prompt = v["prompt_eval_count"].as_u64();
            let generated = v["eval_count"].as_u64();
            let duration = v["eval_duration"].as_u64();
            let pd = v["prompt_eval_duration"].as_u64();
            on(Update::Usage(Usage {
                context_used: prompt.zip(generated).map(|(a, b)| a + b),
                context_limit: m.context,
                prompt_tokens: prompt,
                generated_tokens: generated,
                generation_duration_ns: duration,
                prompt_duration_ns: pd,
                tokens_per_second: generated
                    .zip(duration)
                    .filter(|(_, d)| *d > 0)
                    .map(|(n, d)| n as f64 * 1e9 / d as f64),
                prompt_tokens_per_second: prompt
                    .zip(pd)
                    .filter(|(_, d)| *d > 0)
                    .map(|(n, d)| n as f64 * 1e9 / d as f64),
            }));
        }
        Ok(())
    })
    .await?;
    // Length-stopped argument fragments never reach tools.
    if length && !tool_calls.is_empty() {
        return Err(
            "Tool call hit the output limit; no arguments executed. Retry with a smaller call."
                .into(),
        );
    }
    let calls = calls(&json!({"tool_calls":tool_calls}))?;
    let message = message(text, &calls);
    Ok(Turn {
        message,
        calls,
        length,
    })
}
