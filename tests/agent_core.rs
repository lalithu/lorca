use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use lorca::{
    agent::{Status, TaskView},
    core::Core,
    storage::Repository,
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("lorca-agent-core-{}", lorca::types::id()));
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
    slow: bool,
}
async fn chat(State(s): State<Arc<Server>>, Json(v): Json<Value>) -> Json<Value> {
    s.requests.lock().unwrap().push(v.clone());
    let message = if v.get("tools").is_none() {
        json!({"role":"assistant","content":"Hello. I remember our conversation."})
    } else if v["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["role"] == "tool")
    {
        json!({"role":"assistant","content":"Verified file creation from the tool result."})
    } else {
        json!({"role":"assistant","content":"","tool_calls":[{"id":"read","function":{"name":"list_directory","arguments":{"path":"."}}},{"id":"patch","function":{"name":"apply_patch","arguments":{"path":"created.txt","old_text":"","new_text":"verified"}}}]})
    };
    Json(json!({"message":message,"done":true}))
}
async fn setup(
    slow: bool,
) -> (
    Temp,
    Temp,
    Arc<Core>,
    String,
    Arc<Server>,
    tokio::task::JoinHandle<()>,
) {
    let store = Temp::new();
    let workspace = Temp::new();
    let core = Core::new(Repository::new(store.0.clone()).unwrap()).unwrap();
    let server = Arc::new(Server {
        slow,
        ..Default::default()
    });
    let app = Router::new()
        .route(
            "/api/generate",
            post(
                |State(s): State<Arc<Server>>, Json(v): Json<Value>| async move {
                    s.requests.lock().unwrap().push(v.clone());
                    if s.slow {
                        tokio::time::sleep(Duration::from_secs(20)).await;
                    }
                    Json(json!({"model":v["model"],"done":true,"done_reason":"load"}))
                },
            ),
        )
        .route("/api/chat", post(chat))
        .route("/api/ps", get(|| async { Json(json!({"models":[]})) }))
        .with_state(server.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let session = core
        .command(json!({"op":"connect","host":host,"model":"qwen3.5:9b"}))
        .await
        .unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    (store, workspace, core, session, server, task)
}
async fn wait(core: &Core, p: impl Fn(&TaskView) -> bool) -> TaskView {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(v) = core.agent().snapshot()
                && p(&v)
            {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn unified_core_native_ollama_tools_permissions_and_model_lease() {
    let (_store, w, core, session, server, http) = setup(false).await;
    let task=core.command(json!({"op":"agent_start","session":session,"task":"Create a verified file","workspace":w.0})).await.unwrap()["taskId"].as_str().unwrap().to_string();
    let approval = wait(&core, |v| v.approval.is_some())
        .await
        .approval
        .unwrap();
    let other = core
        .command(json!({"op":"connect","model":"qwen3.5:9b"}))
        .await
        .unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(core.command(json!({"op":"agent_decide","session":other,"taskId":task,"approvalId":approval.id,"decision":"allow_once"})).await.is_err());
    core.command(json!({"op":"agent_decide","session":session,"taskId":task,"approvalId":approval.id,"decision":"allow_once"})).await.unwrap();
    let view = wait(&core, |v| !v.status.active()).await;
    assert_eq!(view.status, Status::Completed);
    assert_eq!(
        std::fs::read_to_string(w.0.join("created.txt")).unwrap(),
        "verified"
    );
    let requests = server.requests.lock().unwrap();
    assert_eq!(
        requests.iter().filter(|v| v.get("tools").is_none()).count(),
        1
    );
    let chats: Vec<_> = requests
        .iter()
        .filter(|v| v.get("tools").is_some())
        .collect();
    assert_eq!(chats.len(), 2);
    assert_eq!(chats[0]["stream"], true);
    assert_eq!(chats[0]["keep_alive"], -1);
    let offered: Vec<_> = chats[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        offered,
        vec![
            "apply_patch",
            "exec_command",
            "list_directory",
            "read_file",
            "request_user_input",
            "search_files",
            "tool_search",
            "write_stdin"
        ]
    );
    let messages = chats[1]["messages"].as_array().unwrap();
    assert_eq!(messages[2]["tool_calls"][1]["id"], "patch");
    assert_eq!(messages[4]["tool_call_id"], "patch");
    assert!(
        messages[4]["content"]
            .as_str()
            .unwrap()
            .contains("verified")
    );
    assert!(
        core.snapshot()
            .sessions
            .iter()
            .find(|v| v.id == session)
            .unwrap()
            .models_to_release
            .contains(&"qwen3.5:9b".to_string())
    );
    http.abort();
}
#[tokio::test]
async fn closing_owner_cancels_agent_and_cleans_loading_state() {
    let (_store, w, core, session, server, http) = setup(true).await;
    core.command(json!({"op":"agent_start","session":session,"task":"Inspect","workspace":w.0}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while server.requests.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    core.command(json!({"op":"close","session":session,"unload":false}))
        .await
        .unwrap();
    assert_eq!(
        wait(&core, |v| !v.status.active()).await.status,
        Status::Cancelled
    );
    assert!(core.snapshot().sessions.iter().all(|v| v.id != session));
    assert!(!w.0.join("created.txt").exists());
    http.abort();
}

#[tokio::test]
async fn model_timeout_clears_preparation_without_closing_the_session() {
    let (_store, w, core, session, _server, http) = setup(true).await;
    core.command(json!({"op":"agent_start","session":session,"task":"Inspect","workspace":w.0,"limits":{"modelSeconds":1}})).await.unwrap();
    assert_eq!(
        wait(&core, |v| !v.status.active()).await.status,
        Status::Failed
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let view = core.snapshot();
            let client = view.sessions.iter().find(|s| s.id == session).unwrap();
            if client.loading_model.is_none() {
                assert!(client.active_model.is_none());
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    core.command(json!({"op":"close","session":session}))
        .await
        .unwrap();
    http.abort();
}

#[tokio::test]
async fn chat_agents_are_idempotent_follow_the_origin_and_persist_only_request_and_result() {
    let (store, w, core, session, _server, http) = setup(false).await;
    let chat = core.snapshot().sessions[0].selected_id.clone();
    let request = lorca::types::id();
    core.command(json!({"op":"draft","session":session,"text":"Create a file"}))
        .await
        .unwrap();
    let command = json!({"op":"agent_start","session":session,"task":"Create a file","conversationId":chat,"requestId":request,"workspace":w.0});
    let started = core.command(command.clone()).await.unwrap();
    assert_eq!(
        core.command(command.clone()).await.unwrap()["taskId"],
        started["taskId"]
    );
    let pending = wait(&core, |v| v.approval.is_some()).await;
    assert_eq!(pending.conversation_id.as_deref(), Some(chat.as_str()));
    assert_eq!(pending.request_id.as_deref(), Some(request.as_str()));
    core.maintenance();
    let checkpoint = core.repository.load().unwrap().0;
    assert_eq!(checkpoint[0].messages.len(), 2);
    assert_eq!(
        checkpoint[0].messages[1].extra["agentStatus"],
        "awaiting_approval"
    );
    assert!(
        core.command(json!({"op":"delete","session":session,"conversationId":chat}))
            .await
            .is_err()
    );
    assert!(core.command(json!({"op":"send","session":session,"conversationId":chat,"requestId":lorca::types::id(),"text":"Race"})).await.is_err());
    core.command(json!({"op":"new","session":session}))
        .await
        .unwrap();
    let next = core.snapshot().sessions[0].selected_id.clone();
    assert_ne!(next, chat);
    assert_eq!(
        core.agent().snapshot().unwrap().status,
        Status::AwaitingApproval
    );
    let approval = pending.approval.unwrap();
    core.command(json!({"op":"agent_decide","session":session,"taskId":pending.id,"approvalId":approval.id,"decision":"allow_once"})).await.unwrap();
    wait(&core, |v| !v.status.active()).await;
    core.maintenance();
    let snapshot = core.snapshot();
    let original = snapshot
        .conversations
        .iter()
        .find(|c| c.id == chat)
        .unwrap();
    assert_eq!(original.messages.len(), 2);
    assert!(original.draft.is_empty());
    assert_eq!(original.messages[1].extra["agentStatus"], "completed");
    assert!(
        snapshot
            .conversations
            .iter()
            .find(|c| c.id == next)
            .unwrap()
            .messages
            .is_empty()
    );
    let recovered = Core::new(Repository::new(store.0.clone()).unwrap()).unwrap();
    let restored = recovered.snapshot();
    let original = restored
        .conversations
        .iter()
        .find(|c| c.id == chat)
        .unwrap();
    assert_eq!(original.messages.len(), 2);
    assert!(original.messages[1].content.contains("Verified"));
    assert!(!original.extra.contains_key("events"));
    assert!(recovered.agent().snapshot().is_none());
    assert_eq!(
        core.command(command).await.unwrap()["taskId"],
        started["taskId"]
    );
    http.abort();
}
#[tokio::test]
async fn cancelled_and_interrupted_chat_agents_preserve_requests_without_restarting() {
    let (store, w, core, session, _server, http) = setup(false).await;
    let chat = core.snapshot().sessions[0].selected_id.clone();
    let command = json!({"op":"agent_start","session":session,"task":"Edit file","conversationId":chat,"requestId":lorca::types::id(),"workspace":w.0});
    core.command(command).await.unwrap();
    let pending = wait(&core, |v| v.approval.is_some()).await;
    // Reconstructing Core from saved data represents a backend restart; no tool is resumed.
    let recovered = Core::new(Repository::new(store.0.clone()).unwrap()).unwrap();
    let state = recovered.snapshot();
    let messages = &state
        .conversations
        .iter()
        .find(|c| c.id == chat)
        .unwrap()
        .messages;
    assert_eq!(messages.len(), 2);
    assert!(messages[1].content.contains("interrupted"));
    assert!(recovered.agent().snapshot().is_none());
    assert!(!w.0.join("created.txt").exists());
    core.command(json!({"op":"agent_cancel","session":session,"taskId":pending.id}))
        .await
        .unwrap();
    wait(&core, |v| !v.status.active()).await;
    core.maintenance();
    assert_eq!(
        core.snapshot().conversations[0].messages[1].extra["agentStatus"],
        "cancelled"
    );
    http.abort();
}

#[tokio::test]
async fn unified_greetings_followups_and_images_preserve_history_without_workspace() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let (store, _workspace, core, session, server, http) = setup(false).await;
    let chat = core
        .snapshot()
        .sessions
        .iter()
        .find(|s| s.id == session)
        .unwrap()
        .selected_id
        .clone();
    for prompt in ["hi", "Explain Rust ownership", "What did I ask earlier?"] {
        let request = lorca::types::id();
        let command = json!({"op":"agent_start","session":session,"conversationId":chat,"requestId":request,"task":prompt});
        let accepted = core.command(command.clone()).await.unwrap();
        let task = wait(&core, |t| !t.status.active()).await;
        assert_eq!(task.status, Status::Completed);
        core.maintenance();
        assert_eq!(
            core.command(command).await.unwrap()["taskId"],
            accepted["taskId"]
        );
    }
    let mut data = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut data)
        .encode(&[255, 0, 0], 1, 1, image::ExtendedColorType::Rgb8)
        .unwrap();
    core.command(json!({"op":"importImage","session":session,"conversationId":chat,"name":"red.jpg","data":STANDARD.encode(data)})).await.unwrap();
    core.command(json!({"op":"agent_start","session":session,"conversationId":chat,"requestId":lorca::types::id(),"task":"Explain this image"})).await.unwrap();
    assert_eq!(
        wait(&core, |t| !t.status.active()).await.status,
        Status::Completed
    );
    core.maintenance();
    {
        let requests = server.requests.lock().unwrap();
        let models = requests
            .iter()
            .filter(|r| r["messages"].is_array())
            .collect::<Vec<_>>();
        assert_eq!(models.len(), 4);
        assert!(models.iter().all(|r| r.get("tools").is_none()));
        assert!(
            models[2]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["content"] == "hi")
        );
        assert!(
            models[2]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["content"] == "Hello. I remember our conversation.")
        );
        assert!(
            models[3]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["images"].is_array())
        );
    }
    let restored = Core::new(Repository::new(store.0.clone()).unwrap()).unwrap();
    let snapshot = restored.snapshot();
    let chat = snapshot
        .conversations
        .iter()
        .find(|c| c.id == chat)
        .unwrap();
    assert_eq!(chat.messages.len(), 8);
    assert_eq!(chat.messages[6].attachments.len(), 1);
    assert!(
        chat.messages[7]
            .extra
            .get("agentHistory")
            .unwrap()
            .is_array()
    );
    http.abort();
}

#[test]
fn restart_keeps_a_tool_only_checkpoint_and_marks_it_interrupted() {
    use lorca::types::{Conversation, Message};
    let temp = Temp::new();
    let repository = Repository::new(temp.0.clone()).unwrap();
    let mut chat = Conversation::default();
    let mut request = Message::new("user", "Edit file".into());
    request.extra.insert("agentKind".into(), json!("request"));
    request
        .extra
        .insert("agentTaskId".into(), json!("interrupted"));
    let mut response = Message::new("assistant", String::new());
    response.extra.insert("agentKind".into(), json!("result"));
    response
        .extra
        .insert("agentTaskId".into(), json!("interrupted"));
    response
        .extra
        .insert("agentStatus".into(), json!("running"));
    let history = json!([{"role":"assistant","tool_calls":[{"id":"patch","function":{"name":"apply_patch","arguments":{"path":"a"}}}]},{"role":"tool","tool_name":"apply_patch","tool_call_id":"patch","content":"{\"ok\":true,\"path\":\"a\"}"}]);
    response
        .extra
        .insert("agentHistory".into(), history.clone());
    chat.messages = vec![request, response];
    repository.save(&chat).unwrap();
    let core = Core::new(repository).unwrap();
    let snapshot = core.snapshot();
    let restored = snapshot
        .conversations
        .iter()
        .find(|c| c.id == chat.id)
        .unwrap();
    assert_eq!(restored.messages.len(), 2);
    assert_eq!(restored.messages[1].extra["agentHistory"], history);
    assert_eq!(restored.messages[1].extra["agentStatus"], "cancelled");
    assert!(restored.messages[1].content.contains("interrupted"));
    assert!(core.agent().snapshot().is_none());
}

#[test]
fn incomplete_and_denied_saved_results_remain_terminal_on_restart() {
    use lorca::types::{Conversation, Message};
    for status in ["incomplete", "denied"] {
        let temp = Temp::new();
        let repository = Repository::new(temp.0.clone()).unwrap();
        let mut chat = Conversation::default();
        let mut request = Message::new("user", "Update README.md".into());
        request.extra.extend([
            ("agentKind".into(), json!("request")),
            ("agentTaskId".into(), json!("settled")),
        ]);
        let mut response = Message::new("assistant", format!("Task {status}: original reason"));
        response.extra.extend([
            ("agentKind".into(), json!("result")),
            ("agentTaskId".into(), json!("settled")),
            ("agentStatus".into(), json!(status)),
            (
                "agentHistory".into(),
                json!([{"role":"assistant","content":"Let me update it."}]),
            ),
        ]);
        chat.messages = vec![request, response];
        repository.save(&chat).unwrap();
        let core = Core::new(repository).unwrap();
        let snapshot = core.snapshot();
        let restored = snapshot
            .conversations
            .iter()
            .find(|c| c.id == chat.id)
            .unwrap();
        assert_eq!(restored.messages.len(), 2);
        assert_eq!(restored.messages[1].extra["agentStatus"], status);
        assert_eq!(
            restored.messages[1].content,
            format!("Task {status}: original reason")
        );
        assert!(core.agent().snapshot().is_none());
    }
}
