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
    let interpreter = v["tools"]
        .as_array()
        .is_some_and(|a| a.iter().any(|s| s["function"]["name"] == "interpret_task"));
    let message = if interpreter {
        json!({"role":"assistant","tool_calls":[{"id":"intent","function":{"name":"interpret_task","arguments":{"relation":"new","objective":"Say hello","outcomes":[{"kind":"explain","target":null}],"edit_targets":[],"forbidden_targets":[],"read_only":false,"commands_forbidden":true,"clarification":null}}}]})
    } else {
        json!({"role":"assistant","content":"Hello"})
    };
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
    for effort in ["nominal", "off"] {
        core.command(json!({"op":"agent_start","session":session,"conversationId":chat,"requestId":lorca::types::id(),"task":"Say hello","effort":effort})).await.unwrap();
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
        assert_eq!(task.status, lorca::agent::Status::Completed);
        if effort == "nominal" {
            assert_eq!(
                task.thinking.iter().map(|b| b.stage).collect::<Vec<_>>(),
                vec![
                    lorca::agent_types::ThinkingStage::Interpretation,
                    lorca::agent_types::ThinkingStage::Execution
                ]
            );
            assert_eq!(
                task.thinking
                    .iter()
                    .filter(|b| b.text == "LOCAL_REASONING_SENTINEL")
                    .count(),
                2
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
        4,
        "Each mode retains one interpretation and one response request"
    );
    assert!(req[..2].iter().all(|v| v["think"] == true));
    assert!(req[2..].iter().all(|v| v["think"] == false));
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
