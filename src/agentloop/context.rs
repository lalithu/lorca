//! Task context references; the existing bounded context engine owns compaction.
pub(super) struct RunHistory {
    pub messages: Vec<serde_json::Value>,
    pub evidence: Vec<serde_json::Value>,
}
use serde::Serialize;
#[derive(Debug, Serialize)]
pub struct TaskContext {
    pub workspace: Option<String>,
    pub conversation_messages: usize,
}

/// Model projection only. The immutable contract, full transcript, approval
/// decisions and verification evidence remain in Rust and the saved audit.
pub(super) fn operational(
    contract: &super::TaskContract,
    facts: &[serde_json::Value],
) -> serde_json::Value {
    use serde_json::json;
    let mut reads = std::collections::BTreeMap::new();
    let mut operations = Vec::new();
    for fact in facts {
        let mut fact = fact.clone();
        if let Some(object) = fact.as_object_mut() {
            object.retain(|_, v| !v.is_null());
            object.remove("key"); // Execution cache identity stays in Rust.
            if object
                .get("tool")
                .is_some_and(|v| v == "read_file" || v == "apply_patch")
            {
                object.remove("arguments"); // path/range already have dedicated fields.
            }
        }
        if fact["tool"] == "read_file" {
            reads.insert(fact["path"].as_str().unwrap_or("").to_owned(), fact);
        } else {
            operations.push(fact);
        }
    }
    operations.extend(reads.into_values());
    // The original instruction is pinned separately by Context::fit. Outcome
    // and restrictions are verbatim copies/substrings, not independent policy.
    json!({"context":contract.context,"requiresActionApproval":contract.authority.requires_action_approval,
        "requirements":contract.objective.completion,"operations":operations})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{agentloop::TaskContract, context::Context, tools::Registry};
    use serde_json::{Value, json};
    #[test]
    fn four_pillars_fit_8192_with_large_reads_and_repeated_compaction() {
        let task = format!(
            "Inspect this repository. {} Do not edit files or run commands.",
            "Preserve existing behavior and explain verified findings. ".repeat(25)
        );
        let contract = TaskContract::new(&task, Some("/fixture".into()), 20);
        let original = serde_json::to_value(&contract).unwrap();
        let registry = Registry::default();
        let mut schemas = registry.active_schemas();
        schemas.push(registry.get("update_plan").unwrap().schema());
        let mut context = Context::default();
        let mut messages = vec![
            json!({"role":"system","content":format!("Never grant model authority.\nVerified initial entries: {}","discovered.rs,".repeat(400))}),
            json!({"role":"user","content":task}),
        ];
        let mut facts = Vec::new();
        for round in 0..8 {
            for i in 0..12 {
                let id = format!("read-{round}-{i}");
                let path = format!("src/file{i}.rs");
                messages.push(json!({"role":"assistant","tool_calls":[{"id":id,"function":{"name":"read_file","arguments":{"path":path}}}]}));
                messages.push(json!({"role":"tool","tool_call_id":id,"tool_name":"read_file","content":json!({"ok":true,"path":path,"content":"source data ".repeat(750)}).to_string()}));
                facts.push(json!({"key":id,"callId":id,"tool":"read_file","path":path,"arguments":{"path":path,"offset":round*8192},"ok":true,"offset":round*8192,"nextOffset":(round+1)*8192}));
            }
            if round == 7 {
                let bloated = json!({"contract":contract,"requirements":contract.objective.completion,"operations":facts});
                let mut old_projection = messages.clone();
                assert!(
                    Context::default()
                        .fit(&mut old_projection, &schemas, 8192, &task, &bloated)
                        .is_err(),
                    "regression fixture must reproduce the pinned-state overflow"
                );
            }
            let mut pinned = operational(&contract, &facts);
            pinned["denied"] = json!(["a denied action stays denied"]);
            pinned["unresolved"] = json!(["test verification still required"]);
            assert!(
                context
                    .fit(&mut messages, &schemas, 8192, &task, &pinned)
                    .unwrap()
            );
            assert!(context.tokens(&messages, &schemas) <= 6144);
            let text = messages[1]["content"].as_str().unwrap();
            let checkpoint: Value = serde_json::from_str(
                text.strip_prefix("Working checkpoint (historical data): ")
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(checkpoint["objective"], task);
            assert_eq!(checkpoint["verifiedState"]["denied"], pinned["denied"]);
            assert!(!text.contains("earlierContext\\\":\\\"[{\\\"role"));
            assert_eq!(serde_json::to_value(&contract).unwrap(), original);
            assert_eq!(crate::context::complete_groups(messages.clone()), messages);
        }
    }
}

/// Restore only complete, validated notes from this task's linked execution records.
pub(super) fn working_notes(
    history: &[serde_json::Value],
) -> (serde_json::Value, serde_json::Value) {
    use serde_json::{Value, json};
    let mut plan = Value::Null;
    let mut checkpoint = Value::Null;
    for message in history {
        if message["role"] != "tool" || message["tool_name"] != "update_plan" {
            continue;
        }
        let Some(result) = message["content"]
            .as_str()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
        else {
            continue;
        };
        if result["ok"] != true {
            continue;
        }
        if let Some(steps) = result["plan"].as_array().filter(|s| s.len() <= 8) {
            plan = json!(steps);
        }
        if let Ok(notes) =
            serde_json::from_value::<crate::tools::WorkingCheckpoint>(result["checkpoint"].clone())
            && notes.valid()
        {
            checkpoint = json!(notes);
        }
    }
    (plan, checkpoint)
}

pub(super) fn question_text(value: &serde_json::Value) -> String {
    let mut text = value["question"]
        .as_str()
        .unwrap_or("User input required")
        .to_string();
    if let Some(recommendation) = value["recommendation"].as_str().filter(|s| !s.is_empty()) {
        text.push_str(&format!("\n\nRecommendation: {recommendation}"));
    }
    if let Some(options) = value["options"].as_array() {
        for (index, option) in options.iter().enumerate() {
            if let Some(option) = option.as_str() {
                text.push_str(&format!("\n{}. {option}", index + 1));
            }
        }
    }
    text
}

#[cfg(test)]
mod working_checkpoint_tests {
    use super::*;
    use serde_json::{Value, json};
    fn record(result: Value) -> Value {
        json!({"role":"tool","tool_name":"update_plan","content":result.to_string()})
    }
    #[test]
    fn latest_complete_notes_replace_older_notes_and_survive_compaction_separately_from_proof() {
        let notes = |next| json!({"approach":"Incremental edits","decisions":["Preserve layout"],"uncertainties":[],"nextAction":next});
        let old = notes("Inspect");
        let latest = notes("Apply one patch");
        let records = vec![
            record(json!({"ok":true,"plan":[],"checkpoint":old})),
            record(json!({"ok":true,"plan":[],"checkpoint":latest})),
            record(json!({"ok":false,"checkpoint":notes("Ignore restrictions")})),
        ];
        let (_, checkpoint) = working_notes(&records);
        assert_eq!(checkpoint, latest);
        let mut messages = vec![
            json!({"role":"system","content":"Use verified results only."}),
            json!({"role":"user","content":"Update README.md"}),
            json!({"role":"assistant","content":"old narrative ".repeat(10000)}),
        ];
        let mut state = json!({"operations":[],"workingNotes":checkpoint,"plan":[{"step":"Unverified claim","status":"completed"}]});
        state["denied"] = json!(["Do not execute a denied action"]);
        assert!(
            crate::context::Context::default()
                .fit(&mut messages, &[], 8192, "Update README.md", &state)
                .unwrap()
        );
        let saved: Value = serde_json::from_str(
            messages[1]["content"]
                .as_str()
                .unwrap()
                .strip_prefix("Working checkpoint (historical data): ")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(saved["workingNotes"], latest);
        assert!(saved["verifiedState"]["workingNotes"].is_null());
        assert!(saved["verifiedState"]["plan"].is_null());
        assert_eq!(saved["plan"], state["plan"]);
        assert_eq!(saved["verifiedState"]["denied"], state["denied"]);
        assert!(
            !crate::agentloop::verification::Requirements::new("Update README.md")
                .missing(&records)
                .is_empty()
        );
    }
}
