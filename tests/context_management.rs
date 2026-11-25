//! Invariants across allocations, saved checkpoints, and readable projections.
use lorca::{
    inference::{Engine, Model},
    memory::journal::WorkingState,
    sessions::{Manager, complete_groups, estimate},
    storage::Repository,
    types::{Conversation, Message, ModelChoice, ModelRoute, id},
};
use serde_json::json;
fn model(capacity: u64) -> Model {
    Model::new(
        ModelChoice {
            route: ModelRoute {
                provider: Engine::Ollama,
                host: "http://127.0.0.1:1".into(),
            },
            model: "test".into(),
            managed: true,
            context_limit: Some(capacity),
        },
        Some(capacity),
        "off".into(),
    )
    .unwrap()
}
#[tokio::test]
async fn windows_fit_and_preserve_latest_instruction_and_tool_pairs() {
    let mut history =
        vec![json!({"role":"system","content":"Current permission checks are authoritative."})];
    for n in 0..18 {
        history.push(json!({"role":"user","content":format!("Source {n} {}","data ".repeat(40))}));
        history.push(json!({"role":"assistant","content":"","tool_calls":[{"id":format!("c{n}"),"type":"function","function":{"name":"read_file","arguments":{"path":"a"}}}]}));
        history.push(
            json!({"role":"tool","tool_call_id":format!("c{n}"),"content":"result ".repeat(40)}),
        );
    }
    history.push(json!({"role":"user","content":"Inspect only; do not modify files."}));
    for capacity in [2048, 4096, 8192] {
        let window = Manager::default()
            .assemble(&model(capacity), &history, &[], &WorkingState::default())
            .await
            .unwrap();
        assert!(window.compact);
        assert!(
            window
                .messages
                .iter()
                .any(|m| m["content"] == "Inspect only; do not modify files.")
        );
        assert!(
            (estimate(&json!({"messages":window.messages,"tools":[]})) as u64 + window.output)
                <= capacity
        );
        assert_eq!(
            complete_groups(window.messages.clone()),
            window.messages,
            "compaction left broken tool pairs"
        );
    }
}
#[tokio::test]
async fn smaller_model_requests_compaction_of_large_saved_checkpoint() {
    let input =
        json!({"role":"user","content":"Continue the task without repeating completed actions."});
    let history = vec![
        json!({"role":"system","content":"Follow user scope."}),
        input.clone(),
    ];
    let state = WorkingState {
        version: 1,
        state: "Verified progress. ".repeat(1000),
        covered: 1,
        boundary: input,
    };
    let window = Manager::default()
        .assemble(&model(2048), &history, &[], &state)
        .await
        .unwrap();
    assert!(
        window.compact,
        "large checkpoints must be re-compacted after switching models"
    );
}
#[test]
fn storage_keeps_readable_journals_stable_records_and_checkpoint_on_reload() {
    let root = std::env::temp_dir().join(format!("lorca-context-{}", id()));
    let repo = Repository::new(root.clone()).unwrap();
    let mut chat = Conversation::default();
    chat.title = "Readable journal".into();
    chat.messages
        .push(Message::new("user", "Inspect my project".into()));
    let mut response = Message::new("assistant", "Verified response".into());
    response.extra.insert("agentHistory".into(),json!([{"role":"assistant","content":"","tool_calls":[{"id":"c1","function":{"name":"read_file","arguments":{"path":"a"}}}]},{"role":"tool","tool_call_id":"c1","content":json!({"ok":true,"api_key":"not-for-export","output":"original evidence"}).to_string()}]));
    chat.messages.push(response);
    repo.save(&chat).unwrap();
    let records = repo.history_records(&chat).unwrap();
    let directory = repo.journal_directory(&chat).unwrap();
    chat.title = "Renamed".into();
    chat.messages.push(Message::new("user", "Follow up".into()));
    repo.save_working_state(
        &chat.id,
        &WorkingState {
            version: 1,
            state: "Objective: continue".into(),
            covered: 2,
            boundary: records[1].1.clone(),
        },
    )
    .unwrap();
    repo.save(&chat).unwrap();
    assert_eq!(directory, repo.journal_directory(&chat).unwrap());
    assert_eq!(records[0], repo.history_records(&chat).unwrap()[0]);
    let markdown = std::fs::read_to_string(directory.join("journal.md")).unwrap();
    assert!(markdown.contains("original evidence"));
    assert!(!markdown.contains("not-for-export"));
    assert!(
        std::fs::read_to_string(directory.join("checkpoint.md"))
            .unwrap()
            .contains("Objective")
    );
    let repo = Repository::new(root.clone()).unwrap();
    assert_eq!(
        repo.working_state(&chat.id).unwrap().state,
        "Objective: continue"
    );
    assert!(
        repo.wire_messages(&repo.load().unwrap().0[0])
            .unwrap()
            .iter()
            .any(|r| r.to_string().contains("not-for-export")),
        "readable redaction changed the source evidence"
    );
    let other = root.join("custom-journals");
    repo.set_journal_root(other.clone()).unwrap();
    assert_eq!(repo.journal_root().unwrap(), other);
    repo.save(&chat).unwrap();
    assert!(
        repo.journal_directory(&chat)
            .unwrap()
            .join("journal.md")
            .exists()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn llama_uses_provider_template_and_tokenizer_when_available() {
    use axum::{Json, Router, routing::post};
    let app = Router::new()
        .route(
            "/apply-template",
            post(|Json(body): Json<serde_json::Value>| async move {
                assert_eq!(body["messages"][0]["content"], "Count this input");
                Json(json!({"prompt":"provider rendered input"}))
            }),
        )
        .route(
            "/tokenize",
            post(|Json(body): Json<serde_json::Value>| async move {
                assert_eq!(body["content"], "provider rendered input");
                Json(json!({"tokens":[1,2,3,4]}))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut model = model(4096);
    model.choice.route.provider = Engine::LlamaCpp;
    model.choice.route.host = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    assert_eq!(
        model
            .count_tokens(&[json!({"role":"user","content":"Count this input"})], &[])
            .await,
        Some(4)
    );
    server.abort();
}
