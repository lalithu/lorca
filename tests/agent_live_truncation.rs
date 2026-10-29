//! Opt-in real-model replay. The fixture contains saved conversation messages,
//! the original screenshot, and baseline frontend files. All edits are confined
//! to a newly created disposable directory; the source conversation is read-only.
use lorca::{agent::*, context::GenerationBudget, provider::agent::OllamaAgent};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
struct Probe {
    model: OllamaAgent,
    calls: AtomicUsize,
    budgets: Mutex<Vec<(u64, u64)>>,
}
impl Inference for Probe {
    fn capacity(&self) -> FutureResult<'_, Option<u64>> {
        self.model.capacity()
    }
    fn generate<'a>(&'a self, messages: &'a [Value], tools: &'a [Value]) -> FutureResult<'a, Turn> {
        self.model.generate(messages, tools)
    }
    fn generate_budgeted<'a>(
        &'a self,
        messages: &'a [Value],
        tools: &'a [Value],
        mut budget: GenerationBudget,
        updates: &'a (dyn Fn(ModelUpdate) + Send + Sync),
    ) -> FutureResult<'a, Turn> {
        let desired = budget.output_limit;
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            budget.output_limit = 2048;
        }
        self.budgets
            .lock()
            .unwrap()
            .push((desired, budget.output_limit));
        self.model
            .generate_budgeted(messages, tools, budget, updates)
    }
}
struct WorkspaceFixture(PathBuf);
impl Drop for WorkspaceFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[tokio::test]
#[ignore = "Requires an explicit saved replay fixture and installed local Ollama model; edits disposable copies only"]
async fn real_frontend_replay_recovers_from_forced_output_exhaustion() {
    let fixture: Value = serde_json::from_slice(
        &std::fs::read(std::env::var("LORCA_REPLAY_FIXTURE").expect("Set LORCA_REPLAY_FIXTURE"))
            .unwrap(),
    )
    .unwrap();
    let model = std::env::var("LORCA_LIVE_MODEL").expect("Set LORCA_LIVE_MODEL");
    let root = WorkspaceFixture(
        std::env::temp_dir().join(format!("lorca-live-truncation-{}", lorca::types::id())),
    );
    std::fs::create_dir(&root.0).unwrap();
    for name in ["index.html", "README.md", "lalith.txt"] {
        std::fs::write(root.0.join(name), fixture["files"][name].as_str().unwrap()).unwrap();
    }
    let original = std::fs::read_to_string(root.0.join("index.html")).unwrap();
    let probe = Arc::new(Probe {
        model: OllamaAgent {
            client: reqwest::Client::new(),
            host: "http://127.0.0.1:11434".into(),
            model: model.clone(),
        },
        calls: AtomicUsize::new(0),
        budgets: Mutex::new(Vec::new()),
    });
    // Restore the model's original unloaded state when this isolated test loads it.
    let before: Value = probe
        .model
        .client
        .get("http://127.0.0.1:11434/api/ps")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let already_loaded = before["models"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["name"] == model);
    let task = fixture["task"].as_str().unwrap();
    let agent = Arc::new(Agent::default());
    let id = agent
        .submit(
            Submission {
                owner: "live-truncation".into(),
                model: Some(model.clone()),
                task: task.into(),
                workspace: root.0.clone(),
                limits: Limits {
                    iterations: 8,
                    model_seconds: 180,
                    ..Default::default()
                },
                conversation_id: None,
                request_id: None,
                history: fixture["history"].as_array().unwrap().clone(),
            },
            probe.clone(),
            Registry::default(),
        )
        .unwrap();
    let mut approvals = std::collections::HashSet::new();
    let result = tokio::time::timeout(Duration::from_secs(420), async {
        loop {
            let view = agent.snapshot().unwrap();
            if let Some(a) = &view.approval
                && approvals.insert(a.id.clone())
            {
                let permitted = a.tool == "apply_patch" && a.action["path"] == "index.html";
                agent
                    .decide(
                        "live-truncation",
                        &id,
                        &a.id,
                        if permitted {
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
        let _ = agent.cancel("live-truncation", &id);
    }
    if !already_loaded {
        let _ = probe
            .model
            .client
            .post("http://127.0.0.1:11434/api/generate")
            .json(&json!({"model":model,"keep_alive":0}))
            .send()
            .await;
    }
    let view = result.expect("Live replay exceeded its bounded deadline");
    println!(
        "{}",
        json!({"model":model,"status":view.status,"result":view.result,"budgetsDesiredActual":*probe.budgets.lock().unwrap(),"truncations":view.audit.iter().filter(|e|e.kind=="generation_truncated").count(),"approvedEdits":approvals.len(),"modelTurns":probe.calls.load(Ordering::SeqCst)})
    );
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert!(
        view.audit.iter().any(|e| e.kind == "generation_truncated"),
        "The test must actually exercise output exhaustion"
    );
    assert!(!view.audit.iter().any(|e| e.kind == "task_continued"
        || (e.kind == "tool_requested" && e.data["tool"] == "request_user_input")));
    let html = std::fs::read_to_string(root.0.join("index.html")).unwrap();
    assert_ne!(html, original);
    for feature in ["dark", "gradient", "file"] {
        assert!(
            html.to_lowercase().contains(feature),
            "Missing generated feature marker: {feature}"
        );
    }
    if let Ok(directory) = std::env::var("LORCA_LIVE_ARTIFACT_DIR") {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(PathBuf::from(directory).join("index.html"), html).unwrap();
    }
}
