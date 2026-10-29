//! Bounded read recovery. Discovery and retry use the same registered tools and
//! authority gateway as model-authored calls; no filesystem access happens here.
use super::{Agent, Registry, Workspace, authority, types::*};
use serde_json::{Value, json};
use tokio::sync::watch;

pub(super) fn path_key(path: &str) -> Option<String> {
    Some(
        Workspace::path(path)
            .ok()?
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(name) => Some(name.to_string_lossy()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/"),
    )
}

pub(super) async fn read_file(
    agent: &Agent,
    registry: &Registry,
    workspace: &Workspace,
    call: &ToolCall,
    limits: &Limits,
    cancel: watch::Receiver<bool>,
    authority: &authority::Authority,
) -> Result<Value> {
    let path = path_key(call.arguments["path"].as_str().ok_or("Missing read path")?)
        .ok_or("Recovery requires a relative workspace path")?;
    let requested = std::path::Path::new(&path);
    let parent = requested
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."))
        .to_string_lossy();
    let name = requested
        .file_name()
        .ok_or("Expected a file path")?
        .to_string_lossy();
    let mut last_error = String::from("No exact file match in bounded discovery results");
    for (tool, args, entries) in [
        ("list_directory", json!({"path":parent}), "entries"),
        (
            "search_files",
            json!({"path":parent,"query":name}),
            "matches",
        ),
    ] {
        let discovery = ToolCall {
            id: format!("{}:recovery:{tool}", call.id),
            name: tool.into(),
            arguments: args,
        };
        agent.emit("tool_requested",json!({"callId":discovery.id,"tool":tool,"arguments":discovery.arguments,"recoveryOf":call.id}));
        let result = authority::execute(
            agent,
            registry,
            Some(workspace),
            &discovery,
            limits,
            cancel.clone(),
            authority,
        )
        .await;
        let value = match &result {
            Ok(v) => v.clone(),
            Err(error) => json!({"ok":false,"error":error}),
        };
        agent.emit(
            if value["ok"] == true {
                "tool_completed"
            } else {
                "tool_failed"
            },
            json!({"callId":discovery.id,"tool":tool,"result":value,"recoveryOf":call.id}),
        );
        if *cancel.borrow() {
            return Err("Cancelled".into());
        }
        if let Err(error) = result {
            if error == "Cancelled" || error.starts_with("User denied") {
                return Err(error);
            }
            last_error = error;
            continue;
        }
        if value["ok"] != true {
            last_error = value["error"].as_str().unwrap_or("Discovery failed").into();
        }
        let matched = value["ok"] == true
            && value[entries].as_array().is_some_and(|items| {
                items.iter().any(|item| {
                    item["path"].as_str().and_then(path_key).as_deref() == Some(path.as_str())
                        && (entries != "entries" || item["kind"] == "file")
                })
            });
        if !matched {
            continue;
        }
        // Re-prepare the original read (including its offset/length); discovery
        // never substitutes for the tool's reference and capability checks.
        let mut retry = call.clone();
        retry.arguments["path"] = json!(path);
        let mut result = authority::execute(
            agent,
            registry,
            Some(workspace),
            &retry,
            limits,
            cancel,
            authority,
        )
        .await?;
        if result["ok"] == true {
            result["recovery"] = json!({"discoveredWith":tool,"path":path});
        }
        return Ok(result);
    }
    Err(format!(
        "Could not confirm the exact requested file {path}: {last_error}"
    ))
}

/// A real registered question, not an assumed answer or a new plan. Used only
/// once when bounded recovery cannot continue the original objective safely.
pub(super) async fn guidance(
    agent: &Agent,
    registry: &Registry,
    workspace: Option<&Workspace>,
    limits: &Limits,
    cancel: watch::Receiver<bool>,
    authority: &authority::Authority,
    reason: &str,
) -> Result<String> {
    let prompt = format!(
        "I couldn't finish the original request: {}. Completed work is preserved. What should I do next, or what information can you provide to unblock it?",
        bounded(reason, 500).trim_end_matches('.')
    );
    question(
        agent, registry, workspace, limits, cancel, authority, &prompt,
    )
    .await
}

pub(super) async fn question(
    agent: &Agent,
    registry: &Registry,
    workspace: Option<&Workspace>,
    limits: &Limits,
    cancel: watch::Receiver<bool>,
    authority: &authority::Authority,
    question: &str,
) -> Result<String> {
    if registry.get("request_user_input").is_err() {
        return Err(format!("Task incomplete: {question}"));
    }
    let call = ToolCall {
        id: crate::types::id(),
        name: "request_user_input".into(),
        arguments: json!({"question":question}),
    };
    let message = json!({"role":"assistant","content":"","tool_calls":[{"id":call.id,"type":"function","function":{"name":call.name,"arguments":call.arguments}}]});
    agent
        .state
        .lock()
        .unwrap()
        .view
        .as_mut()
        .unwrap()
        .transcript
        .push(message);
    agent.emit(
        "tool_requested",
        json!({"callId":call.id,"tool":call.name,"arguments":call.arguments,"recovery":"guidance"}),
    );
    let result =
        authority::execute(agent, registry, workspace, &call, limits, cancel, authority).await;
    let value = match &result {
        Ok(value) => value.clone(),
        Err(error) => json!({"ok":false,"error":error}),
    };
    agent.emit(
        if value["ok"] == true {
            "tool_completed"
        } else {
            "tool_failed"
        },
        json!({"callId":call.id,"tool":call.name,"result":value}),
    );
    agent.state.lock().unwrap().view.as_mut().unwrap().transcript.push(json!({"role":"tool","tool_name":call.name,"tool_call_id":call.id,"content":value.to_string()}));
    result?;
    if value["ok"] != true || value["needsUserInput"] != true {
        return Err(format!(
            "Task incomplete: clarification tool did not pause for input: {}",
            value["error"]
        ));
    }
    Err(format!(
        "Waiting for input: {}",
        value["question"].as_str().unwrap_or(question)
    ))
}
