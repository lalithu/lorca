//! Deterministic execution fixtures; never contact an inference server.
use lorca::{agent::*, provider::agent::parse_turn};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

const REQUEST: &str = "Take a look at this repo. Give me a report on its purpose, the files present, read the files, report back. Do not implement anything or make any changes";
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("lorca-inspection-{}", lorca::types::id()));
        std::fs::create_dir(&dir).unwrap();
        for (name, content) in [
            ("README.md", "Readme"),
            ("index.html", "<html></html>"),
            ("lalith.txt", "lalith"),
        ] {
            std::fs::write(dir.join(name), content).unwrap();
        }
        Self(dir)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Script {
    turns: Mutex<VecDeque<Value>>,
    calls: Mutex<Vec<(bool, usize)>>,
}
impl Inference for Script {
    fn capacity(&self) -> FutureResult<'_, Option<u64>> {
        Box::pin(async { Ok(Some(32768)) })
    }
    fn generate<'a>(
        &'a self,
        _messages: &'a [Value],
        tools: &'a [Value],
    ) -> FutureResult<'a, Turn> {
        Box::pin(async move {
            self.calls.lock().unwrap().push((
                tools
                    .iter()
                    .any(|t| t["function"]["name"] == "interpret_task"),
                tools.len(),
            ));
            parse_turn(
                self.turns
                    .lock()
                    .unwrap()
                    .pop_front()
                    .ok_or("Unexpected model call")?,
            )
        })
    }
}
fn tool(name: &str, args: Value) -> Value {
    json!({"role":"assistant","tool_calls":[{"id":name,"function":{"name":name,"arguments":args}}]})
}
fn reads(names: &[&str]) -> Value {
    json!({"role":"assistant","tool_calls":names.iter().enumerate().map(|(i,p)|json!({
        "id":format!("read-{i}"),"function":{"name":"read_file","arguments":{"path":p}}})).collect::<Vec<_>>()})
}
async fn run(request: &str, turns: Vec<Value>) -> (TaskView, Arc<Script>, Fixture) {
    run_with_workspace(request, turns, true).await
}

async fn run_with_workspace(
    request: &str,
    turns: Vec<Value>,
    use_workspace: bool,
) -> (TaskView, Arc<Script>, Fixture) {
    let fixture = Fixture::new();
    let script = Arc::new(Script {
        turns: Mutex::new(turns.into()),
        calls: Mutex::new(vec![]),
    });
    let agent = Arc::new(Agent::default());
    agent
        .start(
            "fixture",
            request,
            if use_workspace {
                &fixture.0
            } else {
                std::path::Path::new("")
            },
            Limits::default(),
            script.clone(),
            Registry::default(),
        )
        .unwrap();
    let view = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let view = agent.snapshot().unwrap();
            assert!(
                view.approval.is_none(),
                "Read-only task requested an approval"
            );
            if !view.status.active() {
                break view;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    (view, script, fixture)
}
#[tokio::test]
async fn exact_report_request_finishes_once_without_false_filename_or_extra_calls() {
    {
        let (view,script,fixture)=run(REQUEST,vec![
            tool("list_directory",json!({"path":"."})),
            reads(&["README.md","index.html","lalith.txt"]),
            json!({"role":"assistant","content":"Repository report: three files inspected. No changes made."}),
        ]).await;
        assert_eq!(view.status, Status::Completed, "{:?}", view.result);
        let calls = script.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert!(!calls[0].0);
        assert!(calls[..2].iter().all(|&(_, count)| count > 0));
        assert!(
            calls
                .iter()
                .all(|&(interpreting, count)| !interpreting && count > 0)
        );
        assert!(script.turns.lock().unwrap().is_empty());
        assert!(
            !view
                .events
                .iter()
                .any(|e| matches!(e.kind.as_str(), "task_continued" | "task_waiting_for_input"))
        );
        let contract = view.contract.unwrap();
        assert_eq!(contract["objective"]["completion"]["read_paths"], json!([]));
        assert_eq!(
            contract["objective"]["completion"]["read_collections"][0]["coverage"],
            "all_immediate"
        );
        assert_eq!(contract["authority"]["scope"]["read_only"], true);
        assert_eq!(
            std::fs::read_to_string(fixture.0.join("README.md")).unwrap(),
            "Readme"
        );
        assert!(!fixture.0.join("phantom.md").exists());
        assert_eq!(
            view.events
                .iter()
                .filter(|e| e.kind == "tool_started" && e.data["tool"] == "read_file")
                .count(),
            3
        );
    }
}
#[tokio::test]
async fn explicit_missing_file_is_reported_without_summary_retries() {
    let (view, script, _) = run(
        "Read MISSING.md. Do not edit files.",
        vec![
            tool("read_file", json!({"path":"MISSING.md"})),
            json!({"role":"assistant","content":"The requested file was not found."}),
        ],
    )
    .await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(script.calls.lock().unwrap().len(), 2);
    assert!(!view.events.iter().any(|e| e.kind == "task_continued"));
    assert!(
        view.events.iter().any(
            |event| event.kind == "tool_failed" && event.data["result"]["path"] == "MISSING.md"
        )
    );
}
#[tokio::test]
async fn partial_collection_is_recorded_without_reopening_the_reply() {
    let (view, script, _) = run(
        REQUEST,
        vec![
            tool("list_directory", json!({"path":"."})),
            reads(&["README.md"]),
            json!({"role":"assistant","content":"I read README.md."}),
        ],
    )
    .await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    let calls = script.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|&(_, count)| count > 0));
    let report = view
        .events
        .iter()
        .find(|event| event.kind == "task_execution_status")
        .unwrap();
    assert_eq!(report.data["semanticValidationPerformed"], false);
    assert_eq!(report.data["unresolvedOperations"], json!([]));
}

#[tokio::test]
async fn the_model_can_read_remaining_excerpts_and_finish() {
    let (view, script, _) = run(
        REQUEST,
        vec![
            tool("list_directory", json!({"path":"."})),
            reads(&["index.html", "lalith.txt"]),
            tool("read_file", json!({"path":"README.md","length":4})),
            tool("read_file", json!({"path":"README.md","offset":4})),
            json!({"role":"assistant","content":"All three files were inspected."}),
        ],
    )
    .await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    let calls = script.calls.lock().unwrap();
    assert_eq!(calls.len(), 5);
    assert!(calls.iter().all(|&(_, count)| count > 0));
    assert!(
        !view
            .events
            .iter()
            .any(|e| e.kind == "interpretation_rejected")
    );
}

#[tokio::test]
async fn conversation_answers_in_one_request_with_or_without_workspace() {
    for use_workspace in [true, false] {
        let (view, script, _) = run_with_workspace(
            "Can we talk through an idea?",
            vec![json!({"role":"assistant","content":"Of course. What are you thinking?"})],
            use_workspace,
        )
        .await;
        assert_eq!(view.status, Status::Completed, "{:?}", view.result);
        assert_eq!(
            view.result.as_deref(),
            Some("Of course. What are you thinking?")
        );
        let calls = script.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(!calls[0].0 && calls[0].1 > 0);
        assert!(view.contract.is_some());
    }
}

#[tokio::test]
async fn audit_does_not_count_an_excerpt_as_a_full_read() {
    let (view, _, _) = run(
        "Read index.html. Do not edit files.",
        vec![
            tool("read_file", json!({"path":"index.html","length":4})),
            json!({"role":"assistant","content":"The file begins with <htm."}),
        ],
    )
    .await;
    assert_eq!(view.status, Status::Completed);
    assert!(
        view.events
            .iter()
            .any(|e| e.kind == "task_execution_status" && e.data["semanticValidationPerformed"] == false)
    );
    assert!(
        !view
            .events
            .iter()
            .any(|e| e.kind == "finalization_requested")
    );
}

#[tokio::test]
async fn an_answer_without_a_tool_call_is_not_read_evidence() {
    let (view, script, _) = run(
        "Read README.md and tell me what it says.",
        vec![json!({"role":"assistant","content":"The document describes a project."})],
    )
    .await;
    assert_eq!(view.status, Status::Completed);
    assert_eq!(script.calls.lock().unwrap().len(), 1);
    assert!(
        script
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|(interpreting, count)| !interpreting && *count > 0)
    );
    assert!(
        view.events
            .iter()
            .any(|event| event.kind == "task_execution_status" && event.data["semanticValidationPerformed"] == false)
    );
}

#[tokio::test]
async fn raw_no_changes_prohibition_blocks_misclassified_mutation_before_approval() {
    let (view, _, fixture) = run(
        REQUEST,
        vec![
            tool(
                "apply_patch",
                json!({"path":"README.md","old_text":"Readme","new_text":"changed"}),
            ),
            tool(
                "exec_command",
                json!({"executable":"cargo","arguments":["test"]}),
            ),
        ],
    )
    .await;
    assert_eq!(
        std::fs::read_to_string(fixture.0.join("README.md")).unwrap(),
        "Readme"
    );
    let contract = view.contract.unwrap();
    assert_eq!(contract["authority"]["scope"]["read_only"], true);
    assert_eq!(contract["authority"]["scope"]["commands_forbidden"], true);
    assert_eq!(
        view.events
            .iter()
            .filter(|e| e.kind == "tool_failed")
            .count(),
        2
    );
    assert!(
        !view
            .events
            .iter()
            .any(|e| matches!(e.kind.as_str(), "tool_started" | "approval_required"))
    );
}
