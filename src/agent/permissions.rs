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
) -> Result<Value> {
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
