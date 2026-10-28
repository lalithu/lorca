use lorca::{agent::*, provider::agent::parse_turn};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("lorca-reliability-{}", lorca::types::id()));
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("README.md"), "outdated").unwrap();
        std::fs::write(p.join("src/lib.rs"), "pub fn value() -> u32 { 1 }").unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Script {
    turns: Mutex<VecDeque<Value>>,
    seen: Mutex<Vec<(Vec<Value>, usize)>>,
}
impl Script {
    fn new(turns: Vec<Value>) -> Arc<Self> {
        Arc::new(Self {
            turns: Mutex::new(turns.into()),
            seen: Mutex::new(vec![]),
        })
    }
}
impl Inference for Script {
    fn generate<'a>(&'a self, m: &'a [Value], t: &'a [Value]) -> FutureResult<'a, Turn> {
        Box::pin(async move {
            self.seen.lock().unwrap().push((m.to_vec(), t.len()));
            parse_turn(
                self.turns
                    .lock()
                    .unwrap()
                    .pop_front()
                    .ok_or("Unexpected inference request")?,
            )
        })
    }
}
fn text(t: &str) -> Value {
    json!({"role":"assistant","content":t})
}
fn tool(name: &str, args: Value) -> Value {
    json!({"role":"assistant","tool_calls":[{"function":{"name":name,"arguments":args}}]})
}
fn patch(path: &str) -> Value {
    tool(
        "apply_patch",
        json!({"path":path,"old_text":"outdated","new_text":"accurate implementation"}),
    )
}
async fn drive(
    task: &str,
    turns: Vec<Value>,
    decision: Decision,
) -> (TaskView, Arc<Script>, Fixture) {
    drive_in(Fixture::new(), task, turns, decision).await
}
async fn drive_in(
    root: Fixture,
    task: &str,
    turns: Vec<Value>,
    decision: Decision,
) -> (TaskView, Arc<Script>, Fixture) {
    let script = Script::new(turns);
    let agent = Arc::new(Agent::default());
    let id = agent
        .start(
            "test",
            task,
            &root.0,
            Limits::default(),
            script.clone(),
            Registry::default(),
        )
        .unwrap();
    let mut approvals = std::collections::HashSet::new();
    let view = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let v = agent.snapshot().unwrap();
            if let Some(a) = &v.approval
                && approvals.insert(a.id.clone())
            {
                agent.decide("test", &id, &a.id, decision).unwrap();
            }
            if !v.status.active() {
                break v;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    (view, script, root)
}
#[tokio::test]
async fn greeting_and_factual_answers_do_not_trigger_action_recovery() {
    for task in [
        "Hi",
        "Explain Rust ownership",
        "Explain how to update README.md",
        "What did you change and what was the test result? Answer from this conversation without using tools.",
    ] {
        let (v, s, _) = drive(task, vec![text("A factual answer.")], Decision::Deny).await;
        assert_eq!(v.status, Status::Completed);
        assert_eq!(s.seen.lock().unwrap().len(), 1);
    }
}
#[tokio::test]
async fn inspect_requires_file_contents_not_just_listing_or_claims() {
    let (v, _, _) = drive(
        "Inspect this repository",
        vec![
            tool("list_directory", json!({"path":"."})),
            text("I inspected it."),
            text("I'll inspect it"),
            text("Let me inspect it"),
        ],
        Decision::Deny,
    )
    .await;
    assert_eq!(v.status, Status::Incomplete);
    assert_eq!(
        v.events
            .iter()
            .filter(|e| e.kind == "task_continued")
            .count(),
        2
    );
    let (v, _, _) = drive(
        "Inspect this repository",
        vec![
            tool("read_file", json!({"path":"src/lib.rs"})),
            text("src/lib.rs defines value returning 1."),
        ],
        Decision::Deny,
    )
    .await;
    assert_eq!(v.status, Status::Completed);
}
#[tokio::test]
async fn readme_promises_are_bounded_and_do_not_change_files() {
    for response in [
        "I'll update it",
        "Cannot update the workspace?",
        "No changes needed.",
    ] {
        let (v, s, r) = drive(
            "Update README.md",
            vec![text(response), text(response), text(response)],
            Decision::Deny,
        )
        .await;
        assert_eq!(v.status, Status::Incomplete);
        assert_eq!(s.seen.lock().unwrap().len(), 3);
        assert_eq!(
            std::fs::read_to_string(r.0.join("README.md")).unwrap(),
            "outdated"
        );
        assert!(v.result.unwrap().contains("readme.md"));
    }
}
#[tokio::test]
async fn promise_then_real_patch_completes_in_the_same_task() {
    let (v, s, r) = drive(
        "Update README.md",
        vec![
            text("I'll update it"),
            patch("README.md"),
            text("Updated README.md and verified the written bytes."),
        ],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(v.status, Status::Completed);
    assert_eq!(
        std::fs::read_to_string(r.0.join("README.md")).unwrap(),
        "accurate implementation"
    );
    assert!(
        s.seen.lock().unwrap()[1].0.last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("readme.md")
    );
    assert_eq!(
        v.events.iter().filter(|e| e.kind == "file_changes").count(),
        1
    );
}
#[tokio::test]
async fn combined_request_needs_inspection_and_the_requested_patch() {
    let (v, _, _) = drive(
        "Inspect this repository and update README.md",
        vec![
            patch("README.md"),
            text("Updated README.md"),
            text("Done"),
            text("Done"),
        ],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(v.status, Status::Incomplete);
    let (v, _, _) = drive(
        "Inspect this repository and update README.md",
        vec![
            tool("read_file", json!({"path":"src/lib.rs"})),
            patch("README.md"),
            text("README.md now documents the inspected value function."),
        ],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(v.status, Status::Completed);
}
#[tokio::test]
async fn a_different_successful_patch_cannot_satisfy_readme_update() {
    let (v, _, r) = drive(
        "Update README.md",
        vec![
            tool(
                "apply_patch",
                json!({"path":"other.md","old_text":"","new_text":"other"}),
            ),
            text("Done"),
            text("Done"),
            text("Done"),
        ],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(v.status, Status::Incomplete);
    assert_eq!(
        std::fs::read_to_string(r.0.join("README.md")).unwrap(),
        "outdated"
    );
}
#[tokio::test]
async fn empty_after_settled_patch_gets_one_tool_free_finalization_without_replay() {
    let (v, s, _) = drive(
        "Update README.md",
        vec![
            patch("README.md"),
            text(""),
            text("Updated README.md; the patch readback was verified."),
        ],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(v.status, Status::Completed);
    assert_eq!(
        v.events.iter().filter(|e| e.kind == "file_changes").count(),
        1
    );
    let seen = s.seen.lock().unwrap();
    assert_eq!(seen[2].1, 0);
    assert!(
        seen[2].0.last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("Tools are unavailable")
    );
}
#[tokio::test]
async fn empty_finalizer_stays_incomplete_and_never_retries() {
    let (v, s, _) = drive(
        "Update README.md",
        vec![patch("README.md"), text(""), text("")],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(v.status, Status::Incomplete);
    assert_eq!(s.seen.lock().unwrap().len(), 3);
    assert_eq!(
        v.events.iter().filter(|e| e.kind == "file_changes").count(),
        1
    );
}
#[tokio::test]
async fn finalizer_cannot_execute_a_tool_even_if_provider_returns_one() {
    let (v, _, r) = drive(
        "Update README.md",
        vec![
            patch("README.md"),
            text(""),
            tool(
                "apply_patch",
                json!({"path":"unexpected.txt","old_text":"","new_text":"bad"}),
            ),
        ],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(v.status, Status::Incomplete);
    assert!(!r.0.join("unexpected.txt").exists());
}
#[tokio::test]
async fn empty_before_required_edit_continues_with_tools_available() {
    let (v, s, _) = drive(
        "Inspect this repository and update README.md",
        vec![
            tool("read_file", json!({"path":"src/lib.rs"})),
            text(""),
            patch("README.md"),
            text("README.md updated based on src/lib.rs."),
        ],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(v.status, Status::Completed);
    assert!(s.seen.lock().unwrap()[2].1 > 0);
    assert!(!v.events.iter().any(|e| e.kind == "finalization_requested"));
}
#[tokio::test]
async fn denied_patch_is_distinct_and_never_executed() {
    let (v, _, r) = drive(
        "Update README.md",
        vec![patch("README.md"), text("Updated README.md")],
        Decision::Deny,
    )
    .await;
    assert_eq!(v.status, Status::Denied);
    assert!(!v.events.iter().any(|e| e.kind == "tool_started"));
    assert_eq!(
        std::fs::read_to_string(r.0.join("README.md")).unwrap(),
        "outdated"
    );
}
#[tokio::test]
async fn failed_patch_never_turns_a_promise_into_completion() {
    let (v, _, _) = drive(
        "Update README.md",
        vec![
            tool(
                "apply_patch",
                json!({"path":"README.md","old_text":"nonexistent","new_text":"bad"}),
            ),
            text("README.md updated"),
        ],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(v.status, Status::Failed);
    assert!(v.events.iter().any(|e| e.kind == "tool_failed"));
}
#[tokio::test]
async fn guessed_missing_file_recovers_through_real_inventory() {
    let (v, _, _) = drive(
        "Inspect this repository",
        vec![
            tool("read_file", json!({"path":"src/core.rs"})),
            tool("list_directory", json!({"path":"src"})),
            tool("read_file", json!({"path":"src/lib.rs"})),
            text("The source is src/lib.rs, which returns 1."),
        ],
        Decision::Deny,
    )
    .await;
    assert_eq!(v.status, Status::Completed);
    let failure = v.events.iter().find(|e| e.kind == "tool_failed").unwrap();
    assert!(
        failure.data["result"]["error"]
            .as_str()
            .unwrap()
            .contains("src/lib.rs")
    );
}
#[tokio::test]
async fn large_utf8_read_ranges_keep_exact_continuation_metadata() {
    let root = Fixture::new();
    let content = "🦀 Rust ownership\n".repeat(10000);
    std::fs::write(root.0.join("large.txt"), &content).unwrap();
    let w = Workspace::new(&root.0).unwrap();
    let registry = Registry::default();
    let tool = registry.get("read_file").unwrap();
    let mut offset = 0;
    let mut reconstructed = String::new();
    loop {
        let p = tool
            .prepare(
                &w,
                json!({"path":"large.txt","offset":offset,"length":8191}),
                &Limits::default(),
            )
            .unwrap();
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let value = tool
            .execute(&w, &p, &Limits::default(), rx, &mut |_| {})
            .await
            .unwrap();
        assert_eq!(value["offset"], offset);
        reconstructed.push_str(value["content"].as_str().unwrap());
        if let Some(next) = value["nextOffset"].as_u64() {
            assert!(next > offset);
            offset = next;
        } else {
            break;
        }
    }
    assert_eq!(reconstructed, content);
    assert!(
        tool.prepare(
            &w,
            json!({"path":"large.txt","length":0}),
            &Limits::default()
        )
        .is_err()
    );
}

#[tokio::test]
async fn settled_read_only_work_can_finalize_without_tools() {
    let (v, s, _) = drive(
        "Read src/lib.rs",
        vec![
            tool("read_file", json!({"path":"src/lib.rs"})),
            text(""),
            text("src/lib.rs defines value returning 1."),
        ],
        Decision::Deny,
    )
    .await;
    assert_eq!(v.status, Status::Completed);
    assert_eq!(s.seen.lock().unwrap()[2].1, 0);
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn corrected_absolute_command_directory_settles_the_original_preflight_error() {
    let root = Fixture::new();
    std::fs::write(
        root.0.join("Cargo.toml"),
        "[package]\nname=\"lorca_command_recovery\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    )
    .unwrap();
    let directory = root.0.canonicalize().unwrap().to_string_lossy().to_string();
    let (v, _, _) = drive_in(
        root,
        "Run cargo test",
        vec![
            tool(
                "run_command",
                json!({"executable":"cargo","arguments":["test"],"directory":directory}),
            ),
            tool(
                "run_command",
                json!({"executable":"cargo","arguments":["test"]}),
            ),
            text("Corrected the directory to the workspace root. Cargo test ran with exit code 0."),
        ],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(v.status, Status::Completed, "{:?}", v.result);
    assert_eq!(
        v.events.iter().filter(|e| e.kind == "tool_failed").count(),
        1
    );
    assert_eq!(
        v.events
            .iter()
            .filter(|e| e.kind == "approval_required")
            .count(),
        1
    );
    assert_eq!(
        v.events.iter().filter(|e| e.kind == "tool_started").count(),
        1
    );
    assert!(
        v.events
            .iter()
            .any(|e| e.kind == "tool_completed" && e.data["result"]["exitCode"] == 0)
    );
}

#[tokio::test]
async fn task_restriction_blocks_a_model_patch_before_approval_or_side_effect() {
    let (view, _, root) = drive(
        "Inspect README.md; do not edit files",
        vec![patch("README.md"), text("No changes made.")],
        Decision::AllowOnce,
    )
    .await;
    assert_ne!(view.status, Status::Completed);
    assert_eq!(
        std::fs::read_to_string(root.0.join("README.md")).unwrap(),
        "outdated"
    );
    assert!(
        !view
            .events
            .iter()
            .any(|e| e.kind == "approval_required" || e.kind == "file_changes")
    );
    assert!(view.events.iter().any(|e| e.kind == "task_contract"));
}
