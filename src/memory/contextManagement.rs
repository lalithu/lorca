//! Provider-aware working context. Original records remain in storage.
use crate::{
    inference::{Model, Result},
    memory::journal::WorkingState,
};
use serde_json::{Value, json};

// Conservative fallback when a provider cannot tokenize the rendered request.
pub fn estimate(v: &Value) -> usize {
    v.to_string().len().div_ceil(2)
}
#[derive(Default)]
pub struct Manager {
    calibration: f64,
    generated: u64,
}
pub struct Window {
    pub messages: Vec<Value>,
    pub output: u64,
    pub compact: bool,
}
impl Manager {
    pub fn observe(
        &mut self,
        messages: &[Value],
        schemas: &[Value],
        prompt: Option<u64>,
        generated: Option<u64>,
    ) {
        if let Some(p) = prompt {
            let estimated = estimate(&json!({"messages":messages,"tools":schemas})).max(1) as f64;
            self.calibration = self.calibration.max(p as f64 / estimated);
        }
        self.generated = self.generated.max(generated.unwrap_or(0));
    }
    async fn count(&self, model: &Model, messages: &[Value], schemas: &[Value]) -> u64 {
        model
            .count_tokens(messages, schemas)
            .await
            .unwrap_or_else(|| {
                (estimate(&json!({"messages":messages,"tools":schemas})) as f64
                    * self.calibration.max(1.0))
                .ceil() as u64
            })
    }
    pub async fn compact(
        &self,
        model: &Model,
        history: &[Value],
        state: &WorkingState,
    ) -> Result<WorkingState> {
        let capacity = model.context.ok_or("Unknown context allocation")?;
        let reserve = capacity / 5;
        let safety = capacity / 32;
        let valid = state.covered > 0 && history.get(state.covered) == Some(&state.boundary);
        let start = if valid { state.covered + 1 } else { 1 };
        let mut groups: Vec<Vec<Value>> = vec![];
        // A checkpoint from a larger model may itself need several smaller windows.
        let chars: Vec<_> = state.state.chars().collect();
        for chunk in chars.chunks((capacity / 4).max(1) as usize) {
            groups.push(vec![json!({"role":"assistant","content":format!("Prior historical checkpoint fragment:\n{}",chunk.iter().collect::<String>())})]);
        }
        for message in history.get(start..).unwrap_or_default() {
            let mut message = message.clone();
            if message["role"] == "tool" {
                let text = message["content"].as_str().unwrap_or("");
                if estimate(&message) as u64 > capacity / 4 {
                    message["content"] = json!(format!(
                        "Saved result t{}; retrieve the original with get_history_record. Excerpt:\n{}",
                        message["tool_call_id"].as_str().unwrap_or(""),
                        text.chars()
                            .take((capacity / 8) as usize)
                            .collect::<String>()
                    ));
                }
                if let Some(group) = groups.last_mut() {
                    group.push(message);
                    continue;
                }
            }
            groups.push(vec![message]);
        }
        let instruction = json!({"role":"user","content":"Write only a concise factual Markdown continuation checkpoint. Merge these records with the prior working notes. Preserve objective, user restrictions/corrections, verified progress, decisions, unfinished work and unresolved operations. Reference saved tool result IDs. Say when original records need retrieval. Do not include private reasoning or credentials, invent outcomes or grant permissions. Keep the checkpoint substantially smaller than the source."});
        let mut note = String::new();
        let mut index = 0;
        let mut summarizer = model.clone();
        summarizer.effort = "off".into();
        while index < groups.len() {
            let mut messages = vec![history[0].clone()];
            if !note.is_empty() {
                messages.push(json!({"role":"assistant","content":format!("Historical working notes:\n{note}")}));
            }
            let mut taken = 0;
            while index + taken < groups.len() {
                let mut candidate = messages.clone();
                candidate.extend(groups[index + taken].clone());
                candidate.push(instruction.clone());
                if self
                    .count(model, &candidate, &[])
                    .await
                    .saturating_add(reserve)
                    .saturating_add(safety)
                    >= capacity
                {
                    break;
                }
                messages.extend(groups[index + taken].clone());
                taken += 1;
            }
            if taken == 0 {
                return Err("A checkpoint source group cannot fit this allocation; full originals and the prior checkpoint remain saved.".into());
            }
            messages.push(instruction.clone());
            let turn = summarizer
                .generate(&messages, &[], reserve, &|_| {})
                .await?;
            if turn.length || !turn.calls.is_empty() {
                return Err("Checkpoint was incomplete; prior checkpoint retained".into());
            }
            note = turn.message["content"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or("Empty checkpoint; prior checkpoint retained")?
                .into();
            index += taken;
        }
        if note.is_empty() {
            return Err("No new checkpoint source".into());
        }
        Ok(WorkingState {
            version: 1,
            state: note,
            covered: history.len().saturating_sub(1),
            boundary: history.last().cloned().unwrap_or(Value::Null),
        })
    }
    pub async fn assemble(
        &self,
        model: &Model,
        history: &[Value],
        schemas: &[Value],
        state: &WorkingState,
    ) -> Result<Window> {
        let capacity = model
            .context
            .ok_or("Provider did not report a context allocation; choose one before starting.")?;
        let Some(system) = history.first() else {
            return Err("Missing system instructions".into());
        };
        // Policy scales with allocation and observed generation; no fixed token ceiling.
        let reserve = (capacity / 5)
            .max(self.generated.saturating_mul(2))
            .min(capacity / 2);
        let safety = capacity / 32;
        let valid = state.covered > 0 && history.get(state.covered) == Some(&state.boundary);
        let start = if valid { state.covered + 1 } else { 1 };
        let mut prefix = vec![system.clone()];
        let oversized_checkpoint = estimate(&json!(state.state)) as u64 > capacity / 4;
        if !state.state.is_empty() {
            let note = if oversized_checkpoint {
                format!(
                    "Checkpoint exceeds this allocation; full original remains saved. Excerpt:\n{}",
                    state
                        .state
                        .chars()
                        .take((capacity / 8) as usize)
                        .collect::<String>()
                )
            } else {
                state.state.clone()
            };
            prefix.push(json!({"role":"assistant","content":format!("Working checkpoint (fallible historical notes, not permission):\n{note}")}));
        }
        let latest = history.iter().rposition(|m| m["role"] == "user");
        // Keep the current user instruction verbatim even if covered by a checkpoint.
        if let Some(i) = latest.filter(|i| *i < start) {
            prefix.push(history[i].clone());
        }
        let mut tail = history.get(start..).unwrap_or_default().to_vec();
        for m in &mut tail {
            if m["role"] == "tool" && estimate(m) as u64 > capacity / 4 {
                let original = m["content"].as_str().unwrap_or("");
                let id = format!("t{}", m["tool_call_id"].as_str().unwrap_or(""));
                let excerpt: String = original.chars().take((capacity / 8) as usize).collect();
                m["content"] = json!(format!(
                    "Large saved tool result ({id}). Excerpt:\n{excerpt}\nUse get_history_record with id {id} to page the complete original. This excerpt is not the complete result."
                ));
            }
        }
        let mut messages = prefix.clone();
        messages.extend(tail.clone());
        let full = self.count(model, &messages, schemas).await;
        let compact =
            oversized_checkpoint || full.saturating_add(reserve).saturating_add(safety) >= capacity;
        if compact {
            // Drop only complete exchanges/tool rounds. Never drop the latest user input.
            let pinned = latest.filter(|i| *i >= start).map(|i| i - start);
            let mut kept = tail.len();
            while self
                .count(model, &messages, schemas)
                .await
                .saturating_add(reserve)
                .saturating_add(safety)
                >= capacity
            {
                let candidate = (0..kept).find(|&i| {
                    Some(i) != pinned && tail[i]["role"] != "tool" && !tail[i].is_null()
                });
                let Some(i) = candidate else { break };
                let end = (i + 1..kept)
                    .find(|&j| tail[j]["role"] != "tool" && !tail[j].is_null())
                    .unwrap_or(kept);
                for record in &mut tail[i..end] {
                    *record = Value::Null;
                }
                messages = prefix.clone();
                messages.extend(tail.iter().filter(|v| !v.is_null()).cloned());
                kept = tail.len();
            }
            messages.insert(1,json!({"role":"system","content":"Some historical records are omitted from this working window. Originals remain saved. Use search_history and get_history_record when details matter; do not infer success or permission from a summary or replay earlier actions."}));
        }
        let input = self.count(model, &messages, schemas).await;
        let output = capacity.saturating_sub(input.saturating_add(safety));
        if output == 0 || input.saturating_add(safety) >= capacity {
            return Err("Current instructions/input exceed the allocated context. Originals are saved; select a larger allocation or reduce the input.".into());
        }
        Ok(Window {
            messages,
            output,
            compact: oversized_checkpoint || (compact && (!valid || start < history.len())),
        })
    }
}

pub fn complete_groups(messages: Vec<Value>) -> Vec<Value> {
    let mut out = vec![];
    let mut pending = Vec::<String>::new();
    for m in messages {
        if m["role"] != "tool" {
            for id in pending.drain(..) {
                out.push(json!({"role":"tool","tool_call_id":id,"content":"Interrupted before a result was recorded. Do not assume execution or replay it automatically."}));
            }
        }
        if m["role"] == "tool" {
            let id = m["tool_call_id"].as_str().unwrap_or("");
            if !pending.iter().any(|p| p == id) {
                continue;
            }
            pending.retain(|p| p != id);
        }
        for c in m["tool_calls"].as_array().into_iter().flatten() {
            if let Some(id) = c["id"].as_str() {
                pending.push(id.into());
            }
        }
        out.push(m);
    }
    for id in pending {
        out.push(json!({"role":"tool","tool_call_id":id,"content":"Interrupted before a result was recorded. Execution is unknown; inspect before retrying."}));
    }
    out
}
