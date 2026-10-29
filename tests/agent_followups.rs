//! Core persistence and interpretation of completed-task follow-ups.
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use lorca::{agent::Status, core::Core, storage::Repository};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("lorca-followup-{}", lorca::types::id()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[derive(Default)]
struct Server {
    requests: Mutex<Vec<Value>>,
}
fn call(name: &str, args: Value) -> Value {
    json!({"role":"assistant","tool_calls":[{"id":name,"function":{"name":name,"arguments":args}}]})
}
async fn chat(State(s): State<Arc<Server>>, Json(r): Json<Value>) -> Json<Value> {
    s.requests.lock().unwrap().push(r.clone());
    let ms = r["messages"].as_array().unwrap();
    let interpretation = r["tools"]
        .as_array()
        .is_some_and(|ts| ts.iter().any(|t| t["function"]["name"] == "interpret_task"));
    let latest = if interpretation {
        serde_json::from_str::<Value>(ms.last().unwrap()["content"].as_str().unwrap()).unwrap()["latestUserMessage"].as_str().unwrap().to_owned()
    } else {
        ms.iter()
            .rev()
            .filter(|m| m["role"] == "user")
            .filter_map(|m| m["content"].as_str())
            .find(|t| {
                t.contains("Create CHOSEN")
                    || t.contains("Now change")
                    || t.contains("2 + 2")
                    || t.contains("status of")
            })
            .unwrap_or("")
            .into()
    };
    let new = latest.contains("2 + 2");
    let status = latest.contains("status of");
    let follow = latest.contains("Now change");
    let message = if interpretation {
        call(
            "interpret_task",
            json!({"relation":if new {"new"} else if status {"status"} else if follow {"continue"} else {"new"},"objective":if new {"Explain 2 + 2"} else if status {"Report the status of the previous change"} else if follow {"Change CHOSEN.md to second"} else {"Create CHOSEN.md containing first"},"outcomes":[{"kind":if new||status {"explain"}else{"edit"},"target":if new||status {Value::Null}else{json!("CHOSEN.md")}}],"edit_targets":if new||status {json!([])} else {json!(["CHOSEN.md"])},"forbidden_targets":[],"read_only":new||status,"commands_forbidden":true,"clarification":null}),
        )
    } else if new {
        json!({"role":"assistant","content":"2 + 2 = 4."})
    } else if status {
        json!({"role":"assistant","content":"The previous change completed."})
    } else {
        let ok = |name: &str| {
            ms.iter().any(|m| {
                m["role"] == "tool"
                    && m["tool_name"] == name
                    && serde_json::from_str::<Value>(m["content"].as_str().unwrap_or(""))
                        .is_ok_and(|v| v["ok"] == true)
            })
        };
        // Actual tool messages use tool_call_id; both names are stable in this fixture.
        let ok = |name: &str| {
            ok(name)
                || ms.iter().any(|m| {
                    m["role"] == "tool"
                        && m["tool_call_id"] == name
                        && serde_json::from_str::<Value>(m["content"].as_str().unwrap_or(""))
                            .is_ok_and(|v| v["ok"] == true)
                })
        };
        if ok("apply_patch") {
            json!({"role":"assistant","content":"CHOSEN.md updated and verified."})
        } else if follow && !ok("list_directory") {
            call("list_directory", json!({"path":"."}))
        } else if follow && !ok("read_file") {
            call("read_file", json!({"path":"CHOSEN.md"}))
        } else {
            call(
                "apply_patch",
                json!({"path":"CHOSEN.md","old_text":if follow {"first"}else{""},"new_text":if follow {"second"}else{"first"}}),
            )
        }
    };
    Json(json!({"message":message,"done":true,"done_reason":"stop"}))
}
async fn wait(core: &Arc<Core>, session: &str) {
    tokio::time::timeout(Duration::from_secs(12),async {loop {
        core.maintenance();
        let t=core.agent().snapshot().unwrap();
        if let Some(a)=t.approval {
            let rejected=core.command(json!({"op":"agent_start","session":session,"conversationId":t.conversation_id,"workspace":t.workspace,"task":"Replace the active task","requestId":lorca::types::id()})).await;
            assert!(rejected.is_err());
            assert_eq!(core.agent().snapshot().unwrap().id,t.id);
            core.command(json!({"op":"agent_decide","session":session,"taskId":t.id,"approvalId":a.id,"decision":"allow_once"})).await.unwrap();}
        if !t.status.active() {assert_eq!(t.status,Status::Completed,"{:?}",t.result);core.maintenance();return;}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.unwrap();
}
#[tokio::test]
async fn completed_followup_gets_context_but_needs_fresh_evidence_and_status_is_read_only() {
    let data = Temp::new();
    let work = Temp::new();
    let server = Arc::new(Server::default());
    let app = Router::new()
        .route("/api/chat", post(chat))
        .route(
            "/api/generate",
            post(|| async { Json(json!({"done":true,"done_reason":"load"})) }),
        )
        .route(
            "/api/ps",
            get(|| async {
                Json(json!({"models":[{"name":"fixture:latest","context_length":32768}]}))
            }),
        )
        .with_state(server.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let http = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let core = Core::new(Repository::new(data.0.clone()).unwrap()).unwrap();
    let session = core
        .command(json!({"op":"connect","host":host,"model":"fixture"}))
        .await
        .unwrap()["session"]
        .as_str()
        .unwrap()
        .to_owned();
    let conversation = core
        .snapshot()
        .sessions
        .iter()
        .find(|s| s.id == session)
        .unwrap()
        .selected_id
        .clone();
    let mut ids = vec![];
    for prompt in [
        "Create CHOSEN.md containing first. Do not run commands.",
        "Now change its content to second.",
        "What is the status of the previous change?",
        "New request: explain 2 + 2. Do not edit files or run commands.",
    ] {
        core.command(json!({"op":"agent_start","session":session,"model":"fixture","task":prompt,"workspace":work.0,"conversationId":conversation,"requestId":lorca::types::id()})).await.unwrap();
        wait(&core, &session).await;
        let t = core.agent().snapshot().unwrap();
        ids.push(t.id);
        if prompt.starts_with("Now") {
            assert!(
                t.audit
                    .iter()
                    .any(|a| a.kind == "tool_started" && a.data["tool"] == "read_file")
            );
            assert!(
                t.audit
                    .iter()
                    .any(|a| a.kind == "tool_started" && a.data["tool"] == "apply_patch")
            );
        }
        if prompt.contains("status of") {
            assert!(
                !t.audit.iter().any(|a| a.kind == "tool_started"),
                "status must not execute tools"
            );
        }
    }
    assert_eq!(
        std::fs::read_to_string(work.0.join("CHOSEN.md")).unwrap(),
        "second"
    );
    let requests = server.requests.lock().unwrap();
    let interpreted: Vec<Value> = requests
        .iter()
        .filter(|r| {
            r["tools"]
                .as_array()
                .is_some_and(|ts| ts.iter().any(|t| t["function"]["name"] == "interpret_task"))
        })
        .map(|r| {
            serde_json::from_str(
                r["messages"].as_array().unwrap().last().unwrap()["content"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap()
        })
        .collect();
    assert_eq!(interpreted[1]["previousTask"]["completed"], true);
    assert_eq!(interpreted[1]["previousTask"]["id"], ids[0]);
    for m in &core
        .snapshot()
        .conversations
        .iter()
        .find(|c| c.id == conversation)
        .unwrap()
        .messages
    {
        if m.role == "assistant" {
            assert_eq!(m.extra["agentStatus"], "completed");
        }
    }
    http.abort();
}
