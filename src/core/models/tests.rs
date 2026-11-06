use super::*;
use axum::{
    Json, Router,
    routing::{get, post},
};
use provider::Engine::{LlamaCpp, Ollama};

async fn serve(
    router: Router,
    provider: provider::Engine,
) -> (ModelRoute, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (ModelRoute { provider, host }, task)
}

async fn reply(core: &Arc<Core>, session: &str, model: &str, expected: &str) {
    start_and_check_reply(
        core,
        json!({"op":"agent_start","session":session,"task":"Say hello","model":model}),
        expected,
    )
    .await;
}

async fn start_and_check_reply(
    core: &Arc<Core>,
    request: Value,
    expected: &str,
) -> crate::types::Usage {
    core.command(request).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(task) = core.agent.snapshot().filter(|task| !task.status.active()) {
                break task;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(result.status, crate::agent::Status::Completed, "{result:?}");
    assert_eq!(result.result.as_deref(), Some(expected));
    result.usage
}

async fn llama_reply(Json(body): Json<Value>) -> String {
    assert!(matches!(
        body["model"].as_str(),
        Some("same" | "standalone")
    ));
    format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({
            "choices":[{"delta":{"content":"Hello from llama.cpp"},"finish_reason":"stop"}]
        })
    )
}

#[tokio::test]
async fn refresh_and_select_route_duplicate_model_ids_and_retire_each_allocation() {
    let root = std::env::temp_dir().join(format!("lorca-catalog-test-{}", uuid::Uuid::new_v4()));
    let core = Core::new(Repository::new(root.clone()).unwrap()).unwrap();
    let ollama_actions = Arc::new(Mutex::new(Vec::new()));
    let actions = ollama_actions.clone();
    let (ollama, ollama_task) = serve(
        Router::new()
            .route(
                "/api/tags",
                get(|| async { Json(json!({"models":[{"name":"same"}]})) }),
            )
            .route("/api/chat", post(|Json(body): Json<Value>| async move {
                assert_eq!(body["model"], "same");
                Json(json!({"message":{"role":"assistant","content":"Hello from Ollama"},"done":true}))
            }))
            .route(
                "/api/generate",
                post(move |Json(body): Json<Value>| {
                    let actions = actions.clone();
                    async move {
                        actions
                            .lock()
                            .unwrap()
                            .push(body["keep_alive"].as_i64().unwrap());
                        Json(json!({"model":"same","done":true,"done_reason":"unload"}))
                    }
                }),
            ),
        Ollama,
    )
    .await;
    let llama_actions = Arc::new(Mutex::new(Vec::new()));
    let loads = llama_actions.clone();
    let unloads = llama_actions.clone();
    let (llama, llama_task) = serve(
        Router::new()
            .route(
                "/models",
                get(|| async {
                    Json(json!({"data":[{"id":"same","status":{"value":"unloaded"}}]}))
                }),
            )
            .route("/v1/chat/completions", post(llama_reply))
            .route(
                "/models/load",
                post(move || {
                    let loads = loads.clone();
                    async move {
                        loads.lock().unwrap().push("load");
                        Json(json!({"success":true}))
                    }
                }),
            )
            .route(
                "/models/unload",
                post(move || {
                    let unloads = unloads.clone();
                    async move {
                        unloads.lock().unwrap().push("unload");
                        Json(json!({"success":true}))
                    }
                }),
            ),
        LlamaCpp,
    )
    .await;
    let (standalone, standalone_task) = serve(
        Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"standalone"}]})) }),
            )
            .route("/v1/chat/completions", post(llama_reply)),
        LlamaCpp,
    )
    .await;
    let routes = [ollama.clone(), llama.clone(), standalone.clone()];
    let connected = core
        .command(json!({
            "op":"connect","provider":"auto","model":"same",
            "preferredModel":{"route":llama,"model":"same","managed":true}
        }))
        .await
        .unwrap();
    let session = connected["session"].as_str().unwrap();
    core.refresh_local_catalogs(session, &routes).await.unwrap();
    let state = core.snapshot();
    assert_eq!(state.sessions[0].model_choices.len(), 3);
    assert_eq!(
        state.sessions[0].model_choice.as_ref().unwrap().route,
        llama
    );
    assert!(llama_actions.lock().unwrap().is_empty());
    reply(&core, session, "same", "Hello from llama.cpp").await;
    assert_eq!(
        core.snapshot().sessions[0].active_model.as_deref(),
        Some("same")
    );

    for (route, model, expected) in [
        (&ollama, "same", "Hello from Ollama"),
        (&standalone, "standalone", "Hello from llama.cpp"),
    ] {
        core.command(json!({"op":"selectModel","session":session,"model":model,"route":route}))
            .await
            .unwrap();
        reply(&core, session, model, expected).await;
        assert_eq!(
            core.snapshot().sessions[0].active_model.as_deref(),
            Some(model)
        );
    }
    assert_eq!(
        core.snapshot().sessions[0].models_to_release,
        vec!["same", "same"]
    );
    assert!(
        core.command(json!({
            "op":"selectModel","session":session,"model":"same",
            "route":{"provider":"ollama","host":"http://127.0.0.1:12345"}
        }))
        .await
        .is_err()
    );
    assert_eq!(
        core.close(session, true).await.unwrap()["failures"],
        json!([])
    );
    assert_eq!(*ollama_actions.lock().unwrap(), [-1, 0]);
    assert_eq!(*llama_actions.lock().unwrap(), ["load", "unload"]);

    let connected = core
        .command(json!({
            "op":"connect","provider":"auto","model":"same",
            "preferredModel":{"route":llama,"model":"same","managed":true}
        }))
        .await
        .unwrap();
    let session = connected["session"].as_str().unwrap();
    core.refresh_local_catalogs(session, &routes).await.unwrap();
    for task in [llama_task, standalone_task] {
        task.abort();
        let _ = task.await;
    }
    core.refresh_local_catalogs(session, &routes).await.unwrap();
    let state = core.snapshot();
    assert_eq!(state.sessions[0].model_choices.len(), 1);
    assert_eq!(
        state.sessions[0].model_choice.as_ref().unwrap().route,
        ollama
    );
    let (empty, empty_task) = serve(
        Router::new().route("/api/tags", get(|| async { Json(json!({"models":[]})) })),
        Ollama,
    )
    .await;
    core.refresh_local_catalogs(session, &[empty])
        .await
        .unwrap();
    assert!(
        core.snapshot().sessions[0]
            .error
            .as_ref()
            .unwrap()
            .contains("no models available")
    );
    ollama_task.abort();
    let _ = ollama_task.await;
    core.refresh_local_catalogs(session, &routes).await.unwrap();
    let state = core.snapshot();
    assert!(state.sessions[0].model_choices.is_empty());
    assert!(state.sessions[0].model_choice.is_none());
    assert!(
        state.sessions[0]
            .error
            .as_ref()
            .unwrap()
            .contains("Start Ollama or llama.cpp")
    );
    empty_task.abort();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn explicit_ollama_window_reconfigures_the_loaded_model_before_chat() {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    let root = std::env::temp_dir().join(format!("lorca-context-test-{}", uuid::Uuid::new_v4()));
    let core = Core::new(Repository::new(root.clone()).unwrap()).unwrap();
    let allocation = Arc::new(AtomicU64::new(8192));
    let ps = allocation.clone();
    let load = allocation.clone();
    let chat = allocation.clone();
    let (route, server) = serve(
        Router::new()
            .route(
                "/api/tags",
                get(|| async {
                    Json(json!({"models":[{"name":"same","details":{"context_length":40960}}]}))
                }),
            )
            .route(
                "/api/ps",
                get(move || async move {
                    Json(json!({"models":[{"name":"same","context_length":ps.load(Relaxed)}]}))
                }),
            )
            .route(
                "/api/generate",
                post(move |Json(body): Json<Value>| async move {
                    assert_eq!(body["options"]["num_ctx"], 32768);
                    load.store(32768, Relaxed);
                    Json(json!({"model":"same","done":true}))
                }),
            )
            .route(
                "/api/chat",
                post(move |Json(body): Json<Value>| async move {
                    assert_eq!(body["options"]["num_ctx"], 32768);
                    assert_eq!(chat.load(Relaxed), 32768);
                    Json(json!({"message":{"role":"assistant","content":"Hello"},"done":true}))
                }),
            ),
        Ollama,
    )
    .await;
    let connected = core
        .command(json!({"op":"connect","provider":"auto","model":"same"}))
        .await
        .unwrap();
    let session = connected["session"].as_str().unwrap();
    core.refresh_local_catalogs(session, &[route])
        .await
        .unwrap();
    assert_eq!(
        core.snapshot().sessions[0]
            .model_choice
            .as_ref()
            .unwrap()
            .context_limit,
        Some(40960)
    );
    let usage = start_and_check_reply(
        &core,
        json!({"op":"agent_start","session":session,"task":"Say hello","model":"same",
            "contextWindow":32768,"configureContext":true}),
        "Hello",
    )
    .await;
    assert_eq!(usage.context_limit, Some(32768));
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}
