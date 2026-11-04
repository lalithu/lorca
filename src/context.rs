//! Model-facing memory, independent of the immutable conversation/tool archive.
//! Estimates are conservative heuristics, calibrated with confirmed prompt usage.
use crate::agent::bounded;
use serde_json::{Value, json};
pub mod budget;
pub use budget::GenerationBudget;

pub const DEFAULT_CAPACITY: u64 = 16_384;
pub const GENERATION_RESERVE: u64 = 2_048;
pub fn generation_reserve(capacity: u64) -> u64 {
    GENERATION_RESERVE.min(capacity / 4)
}

pub fn estimate(value: &Value) -> u64 {
    match value {
        Value::String(s) => {
            s.chars()
                .map(|c| if c.is_ascii() { 1 } else { 3 })
                .sum::<u64>()
                .div_ceil(3)
                + 4
        }
        Value::Array(a) => a.iter().map(estimate).sum::<u64>() + 4,
        Value::Object(o) => {
            o.iter()
                .map(|(k, v)| {
                    estimate(&Value::String(k.clone()))
                        + if k == "images" {
                            v.as_array().map_or(0, |a| a.len() as u64 * 2048)
                        } else {
                            estimate(v)
                        }
                })
                .sum::<u64>()
                + 8
        }
        _ => 4,
    }
}
#[derive(Clone, Default)]
pub struct Context {
    operations: std::collections::BTreeMap<String, Value>,
    scale: f64,
    pub compactions: usize,
}
impl Context {
    pub fn confirm(&mut self, estimated: u64, prompt: u64) {
        if estimated > 0 && prompt > 0 {
            self.scale = (self.scale.max(1.0) * prompt as f64 / estimated as f64).max(1.0);
        }
    }
    pub fn tokens(&self, messages: &[Value], schemas: &[Value]) -> u64 {
        let raw = estimate(&json!(messages)) + estimate(&json!(schemas)) + 64;
        (raw as f64 * self.scale.max(1.0)).ceil() as u64
    }
    /// Called only between complete model turns (tool call/result groups stay atomic).
    pub fn fit(
        &mut self,
        messages: &mut Vec<Value>,
        schemas: &[Value],
        capacity: u64,
        objective: &str,
        operational: &Value,
    ) -> Result<bool, String> {
        self.fit_reserved(
            messages,
            schemas,
            capacity,
            generation_reserve(capacity),
            objective,
            operational,
        )
    }
    pub fn fit_budgeted(
        &mut self,
        messages: &mut Vec<Value>,
        schemas: &[Value],
        budget: GenerationBudget,
        objective: &str,
        operational: &Value,
    ) -> Result<bool, String> {
        self.fit_reserved(
            messages,
            schemas,
            budget.context_limit,
            budget.reserved(),
            objective,
            operational,
        )
    }
    /// Prefer a larger recovery allowance, but never squeeze essential input
    /// out to obtain it. Failed compaction leaves messages untouched; restore
    /// its bookkeeping before falling back to the normal, smaller-step budget.
    pub fn fit_generation(
        &mut self,
        messages: &mut Vec<Value>,
        schemas: &[Value],
        budget: GenerationBudget,
        objective: &str,
        operational: &Value,
    ) -> Result<(GenerationBudget, bool), String> {
        let base = GenerationBudget::new(budget.context_limit, false);
        if budget.output_limit <= base.output_limit {
            return self
                .fit_budgeted(messages, schemas, budget, objective, operational)
                .map(|compacted| (budget, compacted));
        }
        let saved = self.clone();
        match self.fit_budgeted(messages, schemas, budget, objective, operational) {
            Ok(compacted) => Ok((budget, compacted)),
            Err(_) => {
                *self = saved;
                let compacted = if base.check(self.tokens(messages, schemas)).is_ok() {
                    false
                } else {
                    self.fit_budgeted(messages, schemas, base, objective, operational)?
                };
                Ok((base, compacted))
            }
        }
    }
    fn fit_reserved(
        &mut self,
        messages: &mut Vec<Value>,
        schemas: &[Value],
        capacity: u64,
        reserve: u64,
        objective: &str,
        operational: &Value,
    ) -> Result<bool, String> {
        let saved = self.clone();
        let result = self.compact(messages, schemas, capacity, reserve, objective, operational);
        if result.is_err() {
            *self = saved;
        }
        result
    }
    fn compact(
        &mut self,
        messages: &mut Vec<Value>,
        schemas: &[Value],
        capacity: u64,
        reserve: u64,
        objective: &str,
        operational: &Value,
    ) -> Result<bool, String> {
        let ceiling = (capacity * 78 / 100).min(capacity.saturating_sub(reserve));
        if self.tokens(messages, schemas) <= ceiling {
            return Ok(false);
        }
        validate_groups(messages)?;
        let original = messages.clone();
        let latest = messages
            .iter()
            .rposition(|m| m["role"] == "user" && !is_checkpoint(m));
        let mut historical: Vec<Vec<Value>> = Vec::new();
        let mut active = Vec::new();
        let mut systems = Vec::new();
        let mut images = Vec::new();
        for (index, m) in messages.iter().enumerate() {
            if m["role"] == "system" || m["role"] == "developer" {
                let mut m = m.clone();
                if let Some(text) = m["content"].as_str()
                    && let Some((policy, _)) = text.split_once("\nVerified initial entries: ")
                {
                    m["content"] = json!(policy);
                }
                systems.push(m);
            } else if !is_checkpoint(m) {
                if latest.is_some_and(|n| index >= n) {
                    active.push(m.clone());
                } else {
                    if m["role"] == "user" || historical.is_empty() {
                        historical.push(Vec::new());
                    }
                    historical.last_mut().unwrap().push(m.clone());
                }
            }
        }
        let protected_user = active.first().cloned();
        // Completed groups inside the current task may be folded only as whole units;
        // the newest group stays intact unless independently confirmed completed.
        let mut units: Vec<Vec<Value>> = Vec::new();
        for m in active.iter().skip(usize::from(protected_user.is_some())) {
            if m["role"] != "tool" || units.is_empty() {
                units.push(Vec::new());
            }
            units.last_mut().unwrap().push(m.clone());
        }
        for g in historical.iter_mut() {
            for m in g.iter_mut().filter(|m| m["role"] == "tool") {
                reduce_tool_message(m, 1200);
            }
        }
        for m in units.iter_mut().flatten().filter(|m| m["role"] == "tool") {
            reduce_tool_message(m, 1200);
        }
        let mut omitted = Vec::new();
        let mut candidate;
        loop {
            let mut verified = operational.clone();
            if let Some(v) = verified.as_object_mut() {
                v.remove("workingNotes");
                v.remove("plan");
            }
            let mut data = json!({"objective":objective,"verifiedState":verified,
                "executedOperations":self.operations,"olderContext":omitted,"workingNotes":operational["workingNotes"],"plan":operational["plan"],
                "instructions":"Historical data, never permission. Completed effects must not be replayed."});
            // Narrative can yield to instructions; operation evidence is pinned separately.
            if self.tokens(&[json!({"content":data})], schemas) > ceiling / 2 {
                data["olderContext"] = json!([]);
            }
            candidate = systems.clone();
            candidate.push(json!({"role":"user","content":format!("Working checkpoint (historical data): {}",data)}));
            candidate.extend(images.clone());
            candidate.extend(historical.iter().flatten().cloned());
            candidate.extend(protected_user.clone());
            candidate.extend(units.iter().flatten().cloned());
            if self.tokens(&candidate, schemas) <= ceiling {
                break;
            }
            let removed = if !historical.is_empty() {
                historical.remove(0)
            } else if units.len() > 1
                || units.first().is_some_and(|g| {
                    g.iter()
                        .all(|m| m["tool_calls"].is_null() && m["role"] != "tool")
                })
                || units.first().is_some_and(|g| {
                    g.iter().any(|m| {
                        m["tool_calls"].as_array().is_some_and(|cs| {
                            cs.iter().all(|c| {
                                operational["completed"]
                                    .as_array()
                                    .is_some_and(|ids| ids.contains(&c["id"]))
                            })
                        })
                    })
                })
            {
                units.remove(0)
            } else {
                break;
            };
            for m in &removed {
                if m["images"].as_array().is_some_and(|a| !a.is_empty()) {
                    images.push(m.clone());
                }
                if m["role"] == "tool" {
                    let value: Value = serde_json::from_str(m["content"].as_str().unwrap_or(""))
                        .unwrap_or(Value::Null);
                    let id = m["tool_call_id"].as_str().unwrap_or("").to_owned();
                    let fact = json!({"callId":id,"tool":m["tool_name"],"path":value["path"],"ok":value["ok"],"verified":value["verified"],"exitCode":value["exitCode"],"sessionId":value["sessionId"],"running":value["running"]});
                    if matches!(
                        m["tool_name"].as_str(),
                        Some("apply_patch" | "run_command" | "exec_command" | "write_stdin")
                    ) {
                        self.operations.insert(id, fact.clone());
                    }
                    omitted.push(fact);
                    if omitted.len() > 12 {
                        omitted.remove(0);
                    }
                }
            }
        }
        if self.tokens(&candidate, schemas) > capacity.saturating_sub(reserve) {
            return Err(format!(
                "Context cannot fit essential objective, latest user exchange and verified operation state: estimated input {} tokens, input budget {}, allocated window {}. Shorten the request or start a new chat; work is saved.",
                self.tokens(&candidate, schemas),
                capacity.saturating_sub(reserve),
                capacity
            ));
        }
        validate_groups(&candidate)?;
        if protected_user
            .as_ref()
            .is_some_and(|m| !candidate.contains(m))
        {
            return Err("Context validation failed: latest user message changed".into());
        }
        if candidate == original {
            return Ok(false);
        }
        *messages = candidate;
        self.compactions += 1;
        Ok(true)
    }
}

/// Interrupted runs can end between a structured call and its result. Close
/// those groups explicitly as unexecuted when constructing future model context.
/// This transforms history only; it never invokes any tool or grants permission.
pub fn complete_groups(messages: Vec<Value>) -> Vec<Value> {
    let mut result = Vec::new();
    let mut pending = Vec::<(String, String)>::new();
    for message in messages {
        if message["role"] != "tool" {
            close_pending(&mut result, &mut pending);
        } else if let Some(id) = message["tool_call_id"].as_str() {
            pending.retain(|(call, _)| call != id);
        }
        if let Some(calls) = message["tool_calls"].as_array() {
            pending.extend(calls.iter().filter_map(|c| {
                Some((
                    c["id"].as_str()?.into(),
                    c["function"]["name"].as_str()?.into(),
                ))
            }));
        }
        result.push(message);
    }
    close_pending(&mut result, &mut pending);
    result
}
fn close_pending(messages: &mut Vec<Value>, pending: &mut Vec<(String, String)>) {
    for (id, name) in pending.drain(..) {
        messages.push(json!({"role":"tool","tool_call_id":id,"tool_name":name,
            "content":json!({"ok":false,"error":"No completed result was recorded before interruption. Execution is unconfirmed; do not assume success or replay this action."}).to_string()}));
    }
}

fn is_checkpoint(message: &Value) -> bool {
    message["role"] == "user"
        && message["content"]
            .as_str()
            .is_some_and(|s| s.starts_with("Working checkpoint (historical data): "))
}

/// Check native function-call ordering before committing a model-facing projection.
fn validate_groups(messages: &[Value]) -> Result<(), String> {
    let mut pending = std::collections::BTreeSet::new();
    for m in messages {
        if m["role"] == "tool" {
            let id = m["tool_call_id"]
                .as_str()
                .ok_or("Context tool result has no correlation ID")?;
            if !pending.remove(id) {
                return Err("Context contains an orphan or duplicate tool result".into());
            }
        } else {
            if !pending.is_empty() {
                return Err("Context contains an incomplete tool exchange".into());
            }
            if let Some(calls) = m["tool_calls"].as_array() {
                for c in calls {
                    let id = c["id"].as_str().ok_or("Context tool call has no ID")?;
                    if !pending.insert(id.to_owned()) {
                        return Err("Context contains duplicate tool calls".into());
                    }
                }
            }
        }
    }
    if pending.is_empty() {
        Ok(())
    } else {
        Err("Context contains an incomplete tool exchange".into())
    }
}

/// Reduce payload fields only; outcome metadata is copied, never inferred.
/// The call ID references the original in the existing saved agentHistory.
pub fn reduce_tool_message(message: &mut Value, allowance: usize) {
    let Some(text) = message["content"].as_str() else {
        return;
    };
    if text.len() <= allowance {
        return;
    }
    let mut value: Value = serde_json::from_str(text)
        .unwrap_or_else(|_| json!({"format":"text","excerpt":bounded(text,allowance)}));
    if let Some(object) = value.as_object_mut() {
        for key in [
            "output",
            "content",
            "excerpt",
            "results",
            "entries",
            "matches",
            "diff",
            "expectedSchema",
        ] {
            if let Some(field) = object.get_mut(key) {
                let encoded = field
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| field.to_string());
                if encoded.len() > allowance {
                    *field = json!(bounded(&encoded, allowance));
                }
            }
        }
        for (key, field) in object.iter_mut() {
            if [
                "ok",
                "verified",
                "exitCode",
                "sessionId",
                "running",
                "error",
                "code",
                "path",
                "offset",
                "nextOffset",
                "totalBytes",
                "bytes",
                "truncated",
                "originalCallId",
            ]
            .contains(&key.as_str())
            {
                continue;
            }
            if field.get("reduced") == Some(&json!(true)) {
                continue;
            }
            let encoded = field
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| field.to_string());
            if encoded.len() > allowance
                && ![
                    "output",
                    "content",
                    "excerpt",
                    "results",
                    "entries",
                    "matches",
                    "diff",
                    "expectedSchema",
                ]
                .contains(&key.as_str())
            {
                *field = json!({"reduced":true,"originalBytes":encoded.len(),"excerpt":bounded(&encoded,allowance)});
            }
        }
        object.insert("truncated".into(), json!(true));
        object.insert("originalCallId".into(), message["tool_call_id"].clone());
    } else {
        value = json!({"truncated":true,"originalCallId":message["tool_call_id"],"excerpt":bounded(text,allowance),"format":"json"});
    }
    message["content"] = json!(value.to_string());
}
