use super::{Engine, agent::OllamaAgent};
use crate::{
    agent::{FinishReason, FutureResult, Inference, ModelUpdate, ThinkingEffort, Turn},
    context::GenerationBudget,
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::time::Duration;

/// Thin adapter for llama.cpp's OpenAI-compatible server. It never downloads
/// models or runs llama.cpp itself; the user's server owns model storage/lifecycle.
pub struct LocalAgent {
    pub client: reqwest::Client,
    pub host: String,
    pub model: String,
    pub engine: Engine,
}

impl LocalAgent {
    pub fn generate_for_effort<'a>(
        &'a self,
        messages: &'a [Value],
        tools: &'a [Value],
        budget: GenerationBudget,
        effort: ThinkingEffort,
        updates: &'a (dyn Fn(ModelUpdate) + Send + Sync),
    ) -> FutureResult<'a, Turn> {
        match self.engine {
            Engine::Ollama => Box::pin(async move {
                let provider = OllamaAgent {
                    client: self.client.clone(),
                    host: self.host.clone(),
                    model: self.model.clone(),
                };
                provider
                    .generate_for_effort(messages, tools, budget, effort, updates)
                    .await
            }),
            Engine::LlamaCpp => Box::pin(async move {
                self.generate_llama(messages, tools, budget, effort, updates)
                    .await
            }),
        }
    }

    pub fn generate_stream<'a>(
        &'a self,
        messages: &'a [Value],
        tools: &'a [Value],
        updates: &'a (dyn Fn(ModelUpdate) + Send + Sync),
    ) -> FutureResult<'a, Turn> {
        self.generate_for_effort(
            messages,
            tools,
            GenerationBudget::new(crate::context::DEFAULT_CAPACITY, false),
            ThinkingEffort::Off,
            updates,
        )
    }

    async fn generate_llama(
        &self,
        messages: &[Value],
        tools: &[Value],
        budget: GenerationBudget,
        effort: ThinkingEffort,
        updates: &(dyn Fn(ModelUpdate) + Send + Sync),
    ) -> Result<Turn, String> {
        budget.check(
            crate::context::estimate(&json!(messages))
                + crate::context::estimate(&json!(tools))
                + 64,
        )?;
        let messages = llama_messages(messages);
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
            "stream_options": {"include_usage": true},
            "max_tokens": budget.output_limit,
            "temperature": 0.1,
            "reasoning_effort": match effort {
                ThinkingEffort::Off => "default",
                ThinkingEffort::Nominal => "medium",
                ThinkingEffort::High => "high",
            },
            "reasoning_format": "deepseek",
            "chat_template_kwargs": {"enable_thinking": effort != ThinkingEffort::Off}
        });
        if effort == ThinkingEffort::Off {
            body["reasoning_budget"] = json!(0);
        }
        if !tools.is_empty() {
            body["tools"] = json!(tools.iter().map(openai_tool).collect::<Vec<_>>());
            body["tool_choice"] = json!("auto");
            body["parallel_tool_calls"] = json!(false);
        }
        let requested_reasoning = effort != ThinkingEffort::Off;
        let reason_id = crate::types::id();
        if requested_reasoning {
            updates(ModelUpdate::Reasoning {
                request_id: reason_id.clone(),
                state: "started".into(),
                text: String::new(),
            });
        } else {
            updates(ModelUpdate::Reasoning {
                request_id: reason_id.clone(),
                state: "off".into(),
                text: String::new(),
            });
        }
        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.host))
            .json(&body)
            .timeout(Duration::from_secs(match effort {
                ThinkingEffort::Off => 180,
                ThinkingEffort::Nominal => 240,
                ThinkingEffort::High => 360,
            }))
            .send()
            .await
            .map_err(|e| format!("llama.cpp: {e}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let value: Value = response.json().await.unwrap_or_default();
            return Err(format!(
                "llama.cpp {status}: {}",
                value["error"]["message"]
                    .as_str()
                    .or_else(|| value["error"].as_str())
                    .unwrap_or("request failed")
            ));
        }
        let mut stream = response.bytes_stream();
        let mut pending = Vec::new();
        let mut turn = crate::provider::agent::StreamTurn::default();
        let mut reasoning = String::new();
        let mut usage = None;
        let mut finish = None;
        let mut done = false;
        while let Some(chunk) = stream.next().await {
            pending.extend_from_slice(
                &chunk.map_err(|e| format!("llama.cpp stream interrupted: {e}"))?,
            );
            if pending.len() > 256 * 1024 {
                return Err("llama.cpp stream frame exceeded 256 KiB".into());
            }
            while let Some(end) = pending.iter().position(|b| *b == b'\n') {
                let line: Vec<_> = pending.drain(..=end).collect();
                if consume_sse_line(
                    &line,
                    &mut turn,
                    &mut reasoning,
                    &mut usage,
                    &mut finish,
                    &mut done,
                    &reason_id,
                    requested_reasoning,
                    updates,
                )? {
                    break;
                }
            }
            if done {
                break;
            }
        }
        if !pending.is_empty() {
            consume_sse_line(
                &pending,
                &mut turn,
                &mut reasoning,
                &mut usage,
                &mut finish,
                &mut done,
                &reason_id,
                requested_reasoning,
                updates,
            )?;
        }
        if !done {
            return Err(
                "llama.cpp ended before the completion was confirmed; tool actions were not run"
                    .into(),
            );
        }
        if requested_reasoning {
            updates(ModelUpdate::Reasoning {
                request_id: reason_id,
                state: if reasoning.is_empty() {
                    "empty"
                } else {
                    "complete"
                }
                .into(),
                text: String::new(),
            });
        }
        let usage = usage.unwrap_or_default();
        updates(ModelUpdate::Usage(usage));
        let mut parsed = crate::provider::agent::parse_turn(turn.message()?)?;
        parsed.finish_reason = match finish.as_deref() {
            Some("length") => FinishReason::Length,
            Some("stop" | "tool_calls") => FinishReason::Stop,
            _ => FinishReason::Unknown,
        };
        if parsed.finish_reason == FinishReason::Length {
            parsed.calls.clear();
            parsed.message = json!({"role":"assistant","content":parsed.content});
        }
        Ok(parsed)
    }
}

impl Inference for LocalAgent {
    fn generate<'a>(&'a self, messages: &'a [Value], tools: &'a [Value]) -> FutureResult<'a, Turn> {
        self.generate_stream(messages, tools, &|_| {})
    }
    fn generate_stream<'a>(
        &'a self,
        messages: &'a [Value],
        tools: &'a [Value],
        updates: &'a (dyn Fn(ModelUpdate) + Send + Sync),
    ) -> FutureResult<'a, Turn> {
        LocalAgent::generate_stream(self, messages, tools, updates)
    }
    fn generate_budgeted<'a>(
        &'a self,
        messages: &'a [Value],
        tools: &'a [Value],
        budget: GenerationBudget,
        updates: &'a (dyn Fn(ModelUpdate) + Send + Sync),
    ) -> FutureResult<'a, Turn> {
        self.generate_for_effort(messages, tools, budget, ThinkingEffort::Off, updates)
    }
}

fn openai_tool(value: &Value) -> Value {
    value.clone()
}

fn llama_messages(messages: &[Value]) -> Vec<Value> {
    messages
        .iter()
        .map(|message| {
            let mut message = message.clone();
            if let Some(calls) = message["tool_calls"].as_array_mut() {
                for call in calls {
                    if call["function"]["arguments"].is_object() {
                        call["function"]["arguments"] = json!(call["function"]["arguments"].to_string());
                    }
                }
            }
            if message["role"] == "tool"
                && let Some(name) = message.as_object_mut().and_then(|object| object.remove("tool_name"))
            {
                message["name"] = name;
            }
            if let Some(images) = message["images"].as_array().filter(|images| !images.is_empty()) {
                let text = message["content"].as_str().unwrap_or("").to_owned();
                let mut content = vec![json!({"type":"text","text":text})];
                content.extend(images.iter().filter_map(Value::as_str).map(|data| {
                    json!({"type":"image_url","image_url":{"url":format!("data:image/jpeg;base64,{data}")}})
                }));
                message["content"] = json!(content);
                message.as_object_mut().unwrap().remove("images");
            }
            message
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn consume_sse_line(
    line: &[u8],
    turn: &mut super::agent::StreamTurn,
    reasoning: &mut String,
    usage: &mut Option<crate::types::Usage>,
    finish: &mut Option<String>,
    done: &mut bool,
    request_id: &str,
    report_reasoning: bool,
    updates: &(dyn Fn(ModelUpdate) + Send + Sync),
) -> Result<bool, String> {
    let line = std::str::from_utf8(line)
        .map_err(|_| "Invalid UTF-8 from llama.cpp")?
        .trim();
    let Some(data) = line.strip_prefix("data:").map(str::trim) else {
        return Ok(false);
    };
    if data == "[DONE]" {
        *done = true;
        return Ok(true);
    }
    let value: Value = serde_json::from_str(data).map_err(|_| "Invalid llama.cpp stream frame")?;
    if value["error"].is_object() {
        return Err(value["error"]["message"]
            .as_str()
            .unwrap_or("llama.cpp stream error")
            .into());
    }
    if let Some(prompt) = value["usage"]["prompt_tokens"].as_u64() {
        let generated = value["usage"]["completion_tokens"].as_u64();
        let usage = usage.get_or_insert_with(Default::default);
        usage.prompt_tokens = Some(prompt);
        usage.generated_tokens = generated;
        usage.context_used = generated.map(|n| prompt.saturating_add(n));
    }
    if let Some(ms) = value["timings"]["prompt_ms"]
        .as_f64()
        .filter(|n| n.is_finite() && *n > 0.0)
    {
        let usage = usage.get_or_insert_with(Default::default);
        usage.prompt_duration_ns = Some((ms * 1_000_000.0) as u64);
        usage.prompt_tokens_per_second = value["timings"]["prompt_per_second"]
            .as_f64()
            .filter(|n| n.is_finite() && *n > 0.0);
    }
    if let Some(ms) = value["timings"]["predicted_ms"]
        .as_f64()
        .filter(|n| n.is_finite() && *n > 0.0)
    {
        let usage = usage.get_or_insert_with(Default::default);
        usage.generation_duration_ns = Some((ms * 1_000_000.0) as u64);
        usage.tokens_per_second = value["timings"]["predicted_per_second"]
            .as_f64()
            .filter(|n| n.is_finite() && *n > 0.0)
            .or_else(|| usage.generated_tokens.map(|n| n as f64 * 1000.0 / ms));
    }
    let choice = &value["choices"][0];
    if let Some(reason) = choice["finish_reason"].as_str() {
        *finish = Some(reason.to_owned());
    }
    let delta = &choice["delta"];
    if report_reasoning
        && let Some(text) = delta["reasoning_content"]
            .as_str()
            .filter(|s| !s.is_empty())
    {
        reasoning.push_str(text);
        updates(ModelUpdate::Reasoning {
            request_id: request_id.into(),
            state: "delta".into(),
            text: text.into(),
        });
    }
    let calls = delta["tool_calls"].as_array().cloned().unwrap_or_default();
    let update =
        json!({"message":{"content":delta["content"].as_str().unwrap_or(""),"tool_calls":calls}});
    let text = turn.push(&update)?;
    if !text.is_empty() {
        updates(ModelUpdate::Text(text));
    }
    Ok(false)
}
