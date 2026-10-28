use super::{Agent, Limits, Registry, ToolCall, Workspace, types::*};
use serde_json::{Value, json};
use tokio::sync::watch;

/// Central policy gate. The prepared payload is retained by Rust and cannot be
/// modified by the model or the approval client. Tokens are single-use and task scoped.
pub async fn execute(
    agent: &Agent,
    registry: &Registry,
    w: &Workspace,
    call: &ToolCall,
    limits: &Limits,
    cancel: watch::Receiver<bool>,
    authority: &Authority,
) -> Result<Value> {
    validate(&authority.instruction, call)?;
    let tool = registry.get(&call.name)?;
    let prepared = tool.prepare(w, call.arguments.clone(), limits)?;
    if prepared.approval {
        match agent
            .approval(call, prepared.action.clone(), cancel.clone())
            .await?
        {
            Decision::AllowOnce => {}
            Decision::Deny => {
                return Err("User denied this action; do not retry or bypass the denial".into());
            }
            Decision::Cancel => return Err("Cancelled".into()),
        }
    }
    if *cancel.borrow() {
        return Err("Cancelled".into());
    }
    agent.emit(
        "tool_started",
        json!({"callId":call.id,"tool":call.name,"action":prepared.action}),
    );
    let mut output = |data| agent.emit("terminal_output", json!({"callId":call.id,"output":data}));
    let result = tool
        .execute(w, &prepared, limits, cancel, &mut output)
        .await?;
    if call.name == "apply_patch" && result["ok"] == true {
        agent.emit(
            "file_changes",
            json!({"callId":call.id,"change":prepared.action,"verified":true}),
        );
    }
    Ok(result)
}

/// Conservative recognition of explicit restrictions over existing capabilities.
/// Unrecognized language never adds tools or waives the permission gateway.
pub(super) fn validate(task: &str, call: &ToolCall) -> Result<()> {
    let text = task.to_lowercase();
    let blocked = match call.name.as_str() {
        "apply_patch" => [
            "do not edit",
            "do not modify",
            "do not change",
            "don't edit",
            "don't modify",
            "dont edit",
            "never edit",
            "never modify",
            "do not write",
            "without editing",
            "without modifying",
            "read-only",
            "read only",
            "no edits",
        ]
        .iter()
        .any(|s| text.contains(s)),
        "run_command" => [
            "do not run",
            "don't run",
            "dont run",
            "never run",
            "no commands",
            "do not execute",
            "without running",
        ]
        .iter()
        .any(|s| text.contains(s)),
        _ => false,
    };
    if blocked {
        Err("Task restriction forbids this action; approval cannot expand task scope".into())
    } else {
        Ok(())
    }
}

#[derive(Debug, serde::Serialize)]
pub struct Authority {
    #[serde(skip)]
    instruction: String,
    pub requires_action_approval: bool,
}
impl Authority {
    pub fn new(instruction: &str) -> Self {
        Self {
            instruction: instruction.into(),
            requires_action_approval: true,
        }
    }
}
#[cfg(test)]
mod contract_policy_tests {
    use super::*;
    #[test]
    fn explicit_restrictions_cannot_be_overridden_by_tool_arguments() {
        for name in ["apply_patch", "run_command"] {
            let call = ToolCall {
                id: "test".into(),
                name: name.into(),
                arguments: serde_json::json!({"approved":true}),
            };
            assert!(validate("Read only; do not run commands", &call).is_err());
            assert!(validate("Inspect the project", &call).is_ok()); // Still requires normal gateway approval.
        }
    }
}
