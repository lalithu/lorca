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
    fn interprets_tasks(&self) -> bool {
        true
    }
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
fn interpretation(target: Value) -> Value {
    tool(
        "interpret_task",
        json!({"relation":"new",
        // A hallucinated path in a rewritten objective must not become an obligation.
        "objective":"Read phantom.md and report on repository files",
        "outcomes":[{"kind":"inspect","target":"files present","inspection":target},
            {"kind":"explain","target":"repository purpose"}],
        "edit_targets":[],"forbidden_targets":[],"read_only":true,
        "commands_forbidden":true,"clarification":null}),
    )
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
    for target in [
        Value::Null,
        json!({"type":"collection","directory":".","coverage":"all_immediate"}),
        json!({"type":"file","path":"files present"}),
    ] {
        let (view,script,fixture)=run(REQUEST,vec![
            interpretation(target),tool("list_directory",json!({"path":"."})),
            reads(&["README.md","index.html","lalith.txt"]),
            json!({"role":"assistant","content":"Repository report: three files inspected. No changes made."}),
        ]).await;
        assert_eq!(view.status, Status::Completed, "{:?}", view.result);
        let calls = script.calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        assert!(calls[0].0);
        assert!(calls[1..3].iter().all(|&(_, count)| count > 0));
        assert_eq!(calls[3], (false, 0));
        assert!(
            view.events
                .iter()
                .any(|e| e.kind == "finalization_requested")
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
async fn explicit_missing_file_still_blocks_without_summary_retries() {
    let (view, script, _) = run(
        "Read MISSING.md. Do not edit files.",
        vec![
            interpretation(json!({"type":"subject"})),
            tool("read_file", json!({"path":"MISSING.md"})),
            json!({"role":"assistant","content":"The requested file was not found."}),
        ],
    )
    .await;
    assert_eq!(view.status, Status::WaitingForInput, "{:?}", view.result);
    assert_eq!(script.calls.lock().unwrap().len(), 3);
    assert!(!view.events.iter().any(|e| e.kind == "task_continued"));
    assert!(view.result.unwrap().contains("MISSING.md"));
}
#[tokio::test]
async fn reading_one_of_three_files_cannot_complete_the_collection() {
    let (view, script, _) = run(
        REQUEST,
        vec![
            interpretation(Value::Null),
            tool("list_directory", json!({"path":"."})),
            reads(&["README.md"]),
            json!({"role":"assistant","content":"All files read."}),
            json!({"role":"assistant","content":"All files read."}),
            json!({"role":"assistant","content":"All files read."}),
        ],
    )
    .await;
    assert_eq!(view.status, Status::WaitingForInput, "{:?}", view.result);
    let calls = script.calls.lock().unwrap();
    assert_eq!(calls.len(), 6);
    assert!(calls.iter().all(|&(_, count)| count > 0));
    assert!(view.result.as_ref().unwrap().contains("index.html"));
    assert!(view.result.as_ref().unwrap().contains("lalith.txt"));
}

#[tokio::test]
async fn finalization_waits_for_the_last_excerpt() {
    let (view, script, _) = run(
        REQUEST,
        vec![
            json!({"role":"assistant","content":""}),
            interpretation(Value::Null),
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
    assert_eq!(calls.len(), 7);
    assert!(calls[..6].iter().all(|&(_, count)| count > 0));
    assert_eq!(calls[6], (false, 0));
    assert!(view.events.iter().any(|e| {
        e.kind == "interpretation_rejected"
            && e.data["attempt"] == 1
            && e.data["reason"] == "unexpected_tool_calls"
    }));
}

#[tokio::test]
async fn conversational_reply_needs_one_routing_turn_with_or_without_workspace() {
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
        assert_eq!(*script.calls.lock().unwrap(), [(true, 1)]);
        assert!(view.contract.is_none());
    }
}
