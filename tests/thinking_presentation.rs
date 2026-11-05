use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use lorca::{core::Core, storage::Repository};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Default)]
struct Server {
    requests: Mutex<Vec<Value>>,
}
async fn chat(State(s): State<Arc<Server>>, Json(v): Json<Value>) -> String {
    s.requests.lock().unwrap().push(v.clone());
    let messages = v["messages"].as_array().unwrap();
    let always_empty = messages
        .iter()
        .any(|message| message["content"] == "Always empty fixture");
    let reasoning_only = messages
        .last()
        .is_some_and(|message| message["content"] == "Reasoning-only fixture");
    let content = if reasoning_only || always_empty {
        ""
    } else {
        "Hello"
    };
    let message = json!({"role":"assistant","content":content});
    format!(
        "{}\n{}\n{}\n",
        json!({"message":{"role":"assistant","thinking":"LOCAL_REASONING_SENTINEL"},"done":false}),
        json!({"message":message,"done":false}),
        json!({"done":true,"done_reason":"stop","prompt_eval_count":100,"eval_count":50,"eval_duration":100000000})
    )
}
#[tokio::test]
async fn core_thinking_is_ephemeral_and_off_is_fail_closed() {
    let dir = std::env::temp_dir().join(format!("lorca-thinking-{}", lorca::types::id()));
    let s = Arc::new(Server::default());
    let app = Router::new()
        .route("/api/chat", post(chat))
        .route(
            "/api/show",
            post(|| async { Json(json!({"thinking":{"values":[false,true]}})) }),
        )
        .route(
            "/api/generate",
            post(|| async { Json(json!({"done":true})) }),
        )
        .route(
            "/api/ps",
            get(|| async { Json(json!({"models":[{"name":"fixture","context_length":65536}]})) }),
        )
        .with_state(s.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = format!("http://{}", l.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let core = Core::new(Repository::new(dir.clone()).unwrap()).unwrap();
    let connect = core
        .command(json!({"op":"connect","model":"fixture","host":host}))
        .await
        .unwrap();
    let session = connect["session"].as_str().unwrap();
    let chat = core
        .snapshot()
        .sessions
        .iter()
        .find(|s| s.id == session)
        .unwrap()
        .selected_id
        .clone();
    for (instruction, effort, expected_status) in [
        ("Say hello", "nominal", lorca::agent::Status::Completed),
        ("Say hello", "off", lorca::agent::Status::Completed),
        (
            "Reasoning-only fixture",
            "nominal",
            lorca::agent::Status::Completed,
        ),
        (
            "Always empty fixture",
            "nominal",
            lorca::agent::Status::Incomplete,
        ),
    ] {
        core.command(json!({"op":"agent_start","session":session,"conversationId":chat,"requestId":lorca::types::id(),"task":instruction,"effort":effort})).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if core.agent().snapshot().is_some_and(|v| !v.status.active()) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        core.maintenance();
        let task = core.agent().snapshot().unwrap();
        assert_eq!(task.status, expected_status);
        if instruction == "Reasoning-only fixture" {
            assert_eq!(task.result.as_deref(), Some("Hello"));
            assert_eq!(
                task.audit
                    .iter()
                    .filter(|event| event.kind == "public_reply_recovery")
                    .count(),
                1
            );
        }
        if instruction == "Always empty fixture" {
            assert!(task.assistant_text.is_empty());
            assert_eq!(
                task.audit
                    .iter()
                    .filter(|event| event.kind == "public_reply_recovery")
                    .count(),
                1
            );
        }
        if effort == "nominal" {
            assert_eq!(
                task.thinking.iter().map(|b| b.stage).collect::<Vec<_>>(),
                vec![
                    lorca::agent_types::ThinkingStage::Execution;
                    if instruction == "Say hello" { 1 } else { 2 }
                ]
            );
            assert_eq!(
                task.thinking
                    .iter()
                    .filter(|b| b.text == "LOCAL_REASONING_SENTINEL")
                    .count(),
                1
            );
        } else {
            assert!(task.thinking.iter().all(|b| b.text.is_empty()));
        }
        assert!(
            !serde_json::to_string(&task.transcript)
                .unwrap()
                .contains("LOCAL_REASONING_SENTINEL")
        );
        assert!(
            !serde_json::to_string(&task.audit)
                .unwrap()
                .contains("LOCAL_REASONING_SENTINEL")
        );
    }
    let req = s.requests.lock().unwrap();
    assert_eq!(
        req.len(),
        6,
        "Normal replies need one request; missing replies get exactly one recovery"
    );
    for (request, think) in req.iter().zip([true, false, true, false, true, false]) {
        assert_eq!(request["think"], think);
    }
    for index in [3, 5] {
        assert_eq!(req[index]["tools"], req[index - 1]["tools"]);
        assert!(
            req[index]["messages"].as_array().unwrap().last().unwrap()["content"]
                .as_str()
                .unwrap()
                .starts_with("No public answer was received.")
        );
    }
    assert!(req.iter().all(|v| v["options"]["num_ctx"] == 65536));
    for conversation in &core.snapshot().conversations {
        assert!(
            !serde_json::to_string(conversation)
                .unwrap()
                .contains("LOCAL_REASONING_SENTINEL")
        );
    }
    server.abort();
    std::fs::remove_dir_all(dir).unwrap();
}
