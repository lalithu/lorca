//! Opt-in real Ollama checks. All effects are confined to a disposable workspace.
//! LORCA_LIVE_MODEL=qwen3.5:9b-q4_K_M cargo test --test agent_live -- --ignored --nocapture --test-threads=1
use lorca::{agent::*, provider::agent::OllamaAgent};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
async fn run(task: &str) -> (TaskView, Fixture) {
    let model = std::env::var("LORCA_LIVE_MODEL")
        .expect("Set LORCA_LIVE_MODEL to an installed local model");
    let host = "http://127.0.0.1:11434";
    let http = reqwest::Client::new();
    let before: Value = http
        .get(format!("{host}/api/ps"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let already_loaded = before["models"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["name"] == model);
    let root = Fixture(std::env::temp_dir().join(format!("lorca-live-{}", lorca::types::id())));
    std::fs::create_dir(&root.0).unwrap();
    let agent = Arc::new(Agent::default());
    let id = agent
        .start(
            "live-test",
            task,
            &root.0,
            Limits {
                iterations: 8,
                model_seconds: 90,
                ..Default::default()
            },
            Arc::new(OllamaAgent {
                client: http.clone(),
                host: host.into(),
                model: model.clone(),
            }),
            Registry::default(),
        )
        .unwrap();
    let mut decisions = std::collections::HashSet::new();
    let result = tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            let view = agent.snapshot().unwrap();
            if let Some(a) = &view.approval
                && decisions.insert(a.id.clone())
            {
                let request = view
                    .audit
                    .iter()
                    .find(|e| e.kind == "tool_requested" && e.data["callId"] == a.call_id)
                    .unwrap();
                let args = &request.data["arguments"];
                let exact = a.tool == "apply_patch"
                    && args["path"] == "README.md"
                    && args["old_text"] == ""
                    && args["new_text"] == "";
                agent
                    .decide(
                        "live-test",
                        &id,
                        &a.id,
                        if exact {
                            Decision::AllowOnce
                        } else {
                            Decision::Deny
                        },
                    )
                    .unwrap();
            }
            if !view.status.active() {
                break view;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    if result.is_err() {
        let _ = agent.cancel("live-test", &id);
    }
    if !already_loaded {
        let _ = http
            .post(format!("{host}/api/generate"))
            .json(&json!({"model":model,"keep_alive":0}))
            .send()
            .await;
    }
    let view = result.expect("Live model task timed out");
    println!(
        "LIVE {model}: {task}\n{}",
        serde_json::to_string_pretty(&view).unwrap()
    );
    (view, root)
}
#[tokio::test]
#[ignore = "Requires an installed Ollama model and explicitly permits disposable file writes"]
async fn qwen_creates_empty_readme_without_cargo_or_clarification() {
    let (view, root) =
        run("Create an empty README.md; do not populate it or do anything else.").await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(
        std::fs::metadata(root.0.join("README.md")).unwrap().len(),
        0
    );
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 1);
    assert!(!view.audit.iter().any(|e| e.kind == "tool_requested"
        && matches!(
            e.data["tool"].as_str(),
            Some("exec_command" | "run_command" | "request_user_input")
        )));
}
#[tokio::test]
#[ignore = "Requires an installed Ollama model; no file writes approved"]
async fn qwen_asks_one_question_for_an_unspecified_file() {
    let (view, root) = run("Create a file").await;
    assert_eq!(view.status, Status::WaitingForInput, "{:?}", view.result);
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0);
    assert_eq!(
        view.audit
            .iter()
            .filter(|e| e.kind == "tool_completed"
                && e.data["tool"] == "request_user_input"
                && e.data["result"]["needsUserInput"] == true)
            .count(),
        1
    );
    assert!(
        !view
            .audit
            .iter()
            .any(|e| matches!(e.kind.as_str(), "approval_required" | "task_continued"))
    );
    let question = view
        .audit
        .iter()
        .find(|e| e.kind == "tool_completed" && e.data["tool"] == "request_user_input")
        .unwrap();
    let question = question.data["result"]["question"]
        .as_str()
        .unwrap()
        .to_lowercase();
    assert!(
        question.contains("name") || question.contains("filename") || question.contains("path")
    );
}
