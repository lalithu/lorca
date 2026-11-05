//! Real core + HTTP streaming adapter, with deterministic local inference.
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use lorca::{agent::Status, core::Core, storage::Repository};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Default)]
struct Server {
    requests: Mutex<Vec<Value>>,
    loaded: Mutex<Vec<String>>,
    load_requests: std::sync::atomic::AtomicUsize,
}
async fn models(State(s): State<Arc<Server>>) -> Json<Value> {
    let loaded = s.loaded.lock().unwrap();
    let entries = ["fixture", "second"].map(|id| json!({"id":id,"meta":{"n_ctx":8192},"context_length":131072,"status":{"value":if loaded.iter().any(|n|n==id){"loaded"}else{"unloaded"}}}));
    Json(json!({"data":entries}))
}
async fn load(State(s): State<Arc<Server>>, Json(v): Json<Value>) -> Json<Value> {
    s.load_requests
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    s.loaded
        .lock()
        .unwrap()
        .push(v["model"].as_str().unwrap().into());
    Json(json!({"success":true}))
}
async fn unload(State(s): State<Arc<Server>>, Json(v): Json<Value>) -> Json<Value> {
    s.loaded
        .lock()
        .unwrap()
        .retain(|n| n != v["model"].as_str().unwrap());
    Json(json!({"success":true}))
}
async fn chat(State(s): State<Arc<Server>>, Json(v): Json<Value>) -> String {
    s.requests.lock().unwrap().push(v.clone());
    let messages = v["messages"].as_array().unwrap();
    let hello = messages
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .is_some_and(|m| m["content"] == "Say hello");
    let needs_folder = !hello
        && v["tools"].as_array().is_some_and(|ts| {
            ts.iter()
                .any(|t| t["function"]["name"] == "request_workspace")
        });
    let has_read = messages
        .iter()
        .any(|m| m["role"] == "tool" && m["name"] == "read_file");
    let tool = if needs_folder {
        Some(("request_workspace", json!({})))
    } else if !hello && !has_read && v["tools"].is_array() {
        if v["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["function"]["name"] == "read_file")
        {
            Some(("read_file", json!({"path":"README.md"})))
        } else {
            Some(("tool_search", json!({"query":"read_file"})))
        }
    } else {
        None
    };
    let mut lines = vec![];
    let reason = if let Some((name, args)) = tool {
        // Exercise fragmented OpenAI tool arguments and stable indexes.
        let args = args.to_string();
        let mid = args.len() / 2;
        lines.push(json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call","type":"function","function":{"name":name,"arguments":&args[..mid]}}]}}]}));
        lines.push(json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":&args[mid..]}}]}}]}));
        "tool_calls"
    } else {
        for text in if hello {
            vec!["Hel", "lo"]
        } else {
            vec!["README.md contains ", "fixture contents."]
        } {
            lines.push(json!({"choices":[{"delta":{"content":text}}]}));
        }
        "stop"
    };
    lines.push(json!({"choices":[{"delta":{},"finish_reason":reason}],"usage":{"prompt_tokens":100,"completion_tokens":50},"timings":{"prompt_ms":125.0,"prompt_per_second":800.0,"predicted_ms":100.0,"predicted_per_second":500.0}}));
    lines
        .into_iter()
        .map(|l| format!("data: {l}\n\n"))
        .collect::<String>()
        + "data: [DONE]\n\n"
}
async fn wait(core: &Arc<Core>, expected: Status) {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            core.maintenance();
            let t = core.agent().snapshot().unwrap();
            if !t.status.active() {
                assert_eq!(t.status, expected, "{:?}", t.result);
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    core.maintenance();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_folder_continuation_and_model_lifecycle_use_the_same_core() {
    let root = std::env::temp_dir().join(format!("lorca-local-chat-{}", lorca::types::id()));
    let work = root.join("workspace");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("README.md"), "fixture contents").unwrap();
    let s = Arc::new(Server::default());
    s.loaded.lock().unwrap().push("fixture".into());
    let app = Router::new()
        .route("/models", get(models))
        .route("/models/load", post(load))
        .route("/models/unload", post(unload))
        .route(
            "/props",
            get(|| async { Json(json!({"default_generation_settings":{"n_ctx":0}})) }),
        )
        .route("/v1/chat/completions", post(chat))
        .with_state(s.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let http = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let core = Core::new(Repository::new(root.join("data")).unwrap()).unwrap();
    let connected = core
        .command(json!({"op":"connect","provider":"llama_cpp","host":host,"model":"fixture"}))
        .await
        .unwrap();
    let session = connected["session"].as_str().unwrap();
    let conversation = core.snapshot().sessions[0].selected_id.clone();
    for (task, workspace, expected) in [
        ("Say hello", "", Status::Completed),
        (
            "Read README.md. Do not edit files or run commands.",
            "",
            Status::WaitingForInput,
        ),
        (
            "Use the selected folder to continue the task.",
            work.to_str().unwrap(),
            Status::Completed,
        ),
    ] {
        core.command(json!({"op":"agent_start","session":session,"conversationId":conversation,"requestId":lorca::types::id(),"task":task,"workspace":workspace})).await.unwrap();
        wait(&core, expected.clone()).await;
        if expected == Status::WaitingForInput {
            assert_eq!(
                core.agent().snapshot().unwrap().contract.unwrap()["workspaceRequested"],
                true
            );
        }
    }
    assert_eq!(
        s.load_requests.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "Reuse the already-loaded allocation without restarting it"
    );
    let task = core.agent().snapshot().unwrap();
    assert_eq!(task.usage.context_limit, Some(8192));
    assert_eq!(task.usage.prompt_duration_ns, Some(125_000_000));
    assert_eq!(task.usage.prompt_tokens_per_second, Some(800.0));
    assert!(!task.audit.iter().any(|e| e.kind == "approval_required"));
    assert_eq!(
        task.contract.unwrap()["authority"]["scope"]["read_only"],
        true
    );
    {
        let requests = s.requests.lock().unwrap();
        assert_eq!(
            requests[0]["chat_template_kwargs"]["enable_thinking"],
            false
        );
        assert_eq!(requests[0]["reasoning_budget"], 0);
        assert_eq!(requests[0]["reasoning_format"], "deepseek");
        assert_eq!(requests[0]["tools"].as_array().unwrap().len(), 2);
        assert_eq!(
            requests.len(),
            5,
            "one hello, one folder request, one discovery, one read, one final answer"
        );
        assert!(requests.iter().all(|v| {
            !v["tools"]
                .as_array()
                .is_some_and(|ts| ts.iter().any(|t| t["function"]["name"] == "interpret_task"))
        }));
        assert_eq!(task.usage.tokens_per_second, Some(500.0));
    }
    let saved = core
        .snapshot()
        .conversations
        .into_iter()
        .find(|c| c.id == conversation)
        .unwrap();
    assert_eq!(
        saved.messages.iter().filter(|m| m.role == "user").count(),
        3
    );
    assert!(
        saved
            .messages
            .last()
            .unwrap()
            .content
            .contains("fixture contents")
    );
    core.command(json!({"op":"prepare","session":session,"model":"second","unloadPrevious":true}))
        .await
        .unwrap();
    assert_eq!(*s.loaded.lock().unwrap(), ["second"]);
    core.command(json!({"op":"release","session":session,"model":"second"}))
        .await
        .unwrap();
    assert!(s.loaded.lock().unwrap().is_empty());
    http.abort();
    std::fs::remove_dir_all(root).unwrap();
}
