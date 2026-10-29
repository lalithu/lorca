//! End-to-end question continuation with real Core persistence, tool handlers,
//! approval gates and HTTP inference. Model responses are deterministic fixtures.
use axum::response::IntoResponse;
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
        let p = std::env::temp_dir().join(format!("lorca-checkpoint-{}", lorca::types::id()));
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
    interrupt: std::sync::atomic::AtomicBool,
}
fn result(messages: &[Value], id: &str) -> bool {
    messages.iter().any(|m| {
        m["role"] == "tool"
            && m["tool_call_id"] == id
            && serde_json::from_str::<Value>(m["content"].as_str().unwrap_or(""))
                .is_ok_and(|v| v["ok"] == true)
    })
}
fn call(id: &str, name: &str, arguments: Value) -> Value {
    json!({"role":"assistant","content":"","tool_calls":[{"id":id,"function":{"name":name,"arguments":arguments}}]})
}
async fn chat(
    State(server): State<Arc<Server>>,
    Json(request): Json<Value>,
) -> axum::response::Response {
    server.requests.lock().unwrap().push(request.clone());
    let messages = request["messages"].as_array().unwrap();
    let answered = messages.last().is_some_and(|m| {
        m["content"]
            .as_str()
            .unwrap_or("")
            .contains("User decision: Plain text")
    });
    // On subsequent turns the answer remains in the user history.
    let answered = answered
        || messages.iter().any(|m| {
            m["role"] == "user"
                && m["content"]
                    .as_str()
                    .unwrap_or("")
                    .contains("User decision: Plain text")
        });
    if result(messages, "checkpoint")
        && !answered
        && server
            .interrupt
            .swap(false, std::sync::atomic::Ordering::SeqCst)
    {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"fixture server interrupted"})),
        )
            .into_response();
    }
    let message = if !result(messages, "first-edit") {
        call(
            "first-edit",
            "apply_patch",
            json!({"path":"README.md","old_text":"","new_text":"verified first edit"}),
        )
    } else if !result(messages, "discover-plan") {
        call(
            "discover-plan",
            "tool_search",
            json!({"query":"update_plan"}),
        )
    } else if !result(messages, "checkpoint") {
        call(
            "checkpoint",
            "update_plan",
            json!({"steps":[{"step":"Choose notes format","status":"in_progress"}],"checkpoint":{"approach":"Keep the verified README edit","decisions":["Do not run commands"],"uncertainties":["Notes format"],"nextAction":"Ask the user to choose notes format"}}),
        )
    } else if !answered {
        call(
            "question",
            "request_user_input",
            json!({"question":"Which format for notes?","recommendation":"Plain text is sufficient.","options":["Plain text","Markdown"]}),
        )
    } else if !result(messages, "second-edit") {
        call(
            "second-edit",
            "apply_patch",
            json!({"path":"notes.txt","old_text":"","new_text":"chosen plain text"}),
        )
    } else {
        json!({"role":"assistant","content":"README.md and notes.txt were created and verified."})
    };
    Json(json!({"message":message,"done":true,"done_reason":"stop"})).into_response()
}
async fn wait_saved(core: &Arc<Core>, status: Status) {
    tokio::time::timeout(Duration::from_secs(8),async {
        loop {
            core.maintenance();
            let view=core.snapshot();
            if let Some(task)=core.agent().snapshot() {
                if let Some(approval)=&task.approval {
                    core.command(json!({"op":"agent_decide","session":task.owner,"taskId":task.id,"approvalId":approval.id,"decision":"allow_once"})).await.unwrap();
                }
                if task.status==status && view.conversations.iter().any(|c|c.messages.iter().any(|m|m.extra.get("agentTaskId")==Some(&json!(task.id)) && m.extra.get("agentStatus")==Some(&json!(status)))) { return; }
                assert!(task.status.active() || task.status==status,"unexpected terminal state: {:?}: {:?}",task.status,task.result);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
}
async fn connect(core: &Arc<Core>, host: &str) -> String {
    core.command(json!({"op":"connect","host":host,"model":"fixture"}))
        .await
        .unwrap()["session"]
        .as_str()
        .unwrap()
        .to_owned()
}
async fn restart_continuation(interrupted: bool) {
    let store = Temp::new();
    let workspace = Temp::new();
    let server = Arc::new(Server::default());
    server
        .interrupt
        .store(interrupted, std::sync::atomic::Ordering::SeqCst);
    let app = Router::new()
        .route("/api/chat", post(chat))
        .route(
            "/api/generate",
            post(|| async { Json(json!({"done":true,"done_reason":"load"})) }),
        )
        .route("/api/ps", get(|| async { Json(json!({"models":[]})) }))
        .with_state(server.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let http = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let core = Core::new(Repository::new(store.0.clone()).unwrap()).unwrap();
    let session = connect(&core, &host).await;
    let conversation = core
        .snapshot()
        .sessions
        .iter()
        .find(|s| s.id == session)
        .unwrap()
        .selected_id
        .clone();
    core.command(json!({"op":"agent_start","session":session,"conversationId":conversation,"requestId":lorca::types::id(),"workspace":workspace.0,"task":"Create README.md and notes.txt; do not run commands"})).await.unwrap();
    let paused_status = if interrupted {
        Status::Failed
    } else {
        Status::WaitingForInput
    };
    wait_saved(&core, paused_status.clone()).await;
    assert_eq!(
        std::fs::read_to_string(workspace.0.join("README.md")).unwrap(),
        "verified first edit"
    );
    assert!(!workspace.0.join("notes.txt").exists());
    let original = core.agent().snapshot().unwrap();
    drop(core);
    if interrupted {
        // Older chats have the request and transcript but no new objective/workspace metadata on the response.
        let repository = Repository::new(store.0.clone()).unwrap();
        let mut chat = repository
            .load()
            .unwrap()
            .0
            .into_iter()
            .find(|chat| chat.id == conversation)
            .unwrap();
        for message in &mut chat.messages {
            message.extra.remove("agentObjective");
            if message.role == "assistant" {
                message.extra.remove("agentWorkspace");
            }
        }
        repository.save(&chat).unwrap();
    }
    let core = Core::new(Repository::new(store.0.clone()).unwrap()).unwrap();
    let session = connect(&core, &host).await;
    assert!(
        core.snapshot()
            .conversations
            .iter()
            .find(|c| c.id == conversation)
            .unwrap()
            .messages
            .iter()
            .any(|m| m.extra.get("agentStatus") == Some(&json!(paused_status)))
    );
    let other_workspace = Temp::new();
    assert!(core.command(json!({"op":"agent_start","session":session,"conversationId":conversation,"requestId":lorca::types::id(),"workspace":other_workspace.0,"task":"Plain text","continueTaskId":original.id})).await.is_err());
    core.command(json!({"op":"agent_start","session":session,"conversationId":conversation,"requestId":lorca::types::id(),"workspace":workspace.0,"task":"Plain text","continueTaskId":if interrupted { Some(&original.id) } else { None }})).await.unwrap();
    wait_saved(&core, Status::Completed).await;
    let resumed = core.agent().snapshot().unwrap();
    assert_ne!(resumed.id, original.id);
    assert!(resumed.task.contains("do not run commands"));
    assert!(resumed.task.contains("User decision: Plain text"));
    assert_eq!(
        resumed
            .audit
            .iter()
            .filter(|e| e.kind == "file_changes")
            .count(),
        1,
        "first edit must not replay"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.0.join("notes.txt")).unwrap(),
        "chosen plain text"
    );
    assert!(
        resumed.audit.iter().any(|e| e.kind == "approval_required"),
        "an answer must not grant edit permission"
    );
    assert_eq!(server.requests.lock().unwrap().len(), 6);
    http.abort();
}

#[tokio::test]
async fn answering_after_restart_retains_objective_and_verified_work_without_replaying_it() {
    restart_continuation(false).await;
}
#[tokio::test]
async fn explicitly_continuing_after_server_failure_restores_checkpoint_without_waiving_approval() {
    restart_continuation(true).await;
}
