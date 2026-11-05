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
    schemas: Mutex<Vec<Vec<Value>>>,
    capacity: Option<u64>,
}
impl Script {
    fn new(turns: Vec<Value>) -> Arc<Self> {
        Arc::new(Self {
            turns: Mutex::new(turns.into()),
            seen: Mutex::new(vec![]),
            schemas: Mutex::new(vec![]),
            capacity: None,
        })
    }
}
impl Inference for Script {
    fn capacity(&self) -> FutureResult<'_, Option<u64>> {
        Box::pin(async { Ok(self.capacity) })
    }
    fn generate<'a>(&'a self, m: &'a [Value], t: &'a [Value]) -> FutureResult<'a, Turn> {
        Box::pin(async move {
            self.seen.lock().unwrap().push((m.to_vec(), t.len()));
            self.schemas.lock().unwrap().push(t.to_vec());
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
    drive_with_capacity(root, task, turns, decision, None).await
}
async fn drive_with_capacity(
    root: Fixture,
    task: &str,
    turns: Vec<Value>,
    decision: Decision,
    capacity: Option<u64>,
) -> (TaskView, Arc<Script>, Fixture) {
    let mut script = Script::new(turns);
    Arc::get_mut(&mut script).unwrap().capacity = capacity;
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
async fn inspection_audit_distinguishes_a_listing_from_a_read() {
    let (v, script, _) = drive(
        "Inspect this repository",
        vec![
            tool("list_directory", json!({"path":"."})),
            text("I inspected it."),
        ],
        Decision::Deny,
    )
    .await;
    assert_eq!(v.status, Status::Completed);
    assert!(
        v.events
            .iter()
            .any(|event| event.kind == "task_verification" && event.data["satisfied"] == false)
    );
    assert_eq!(script.seen.lock().unwrap().len(), 2);
    let (v, script, _) = drive(
        "Inspect this repository",
        vec![
            tool("read_file", json!({"path":"src/lib.rs"})),
            text("src/lib.rs defines value returning 1."),
        ],
        Decision::Deny,
    )
    .await;
    assert_eq!(v.status, Status::Completed);
    assert_eq!(script.seen.lock().unwrap().len(), 2);
    assert!(
        v.events
            .iter()
            .any(|event| event.kind == "task_verification" && event.data["satisfied"] == true)
    );
}
#[tokio::test]
async fn tool_free_replies_stop_and_do_not_change_files() {
    for response in [
        "I'll update it",
        "Cannot update the workspace?",
        "No changes needed.",
    ] {
        let (v, s, r) = drive("Update README.md", vec![text(response)], Decision::Deny).await;
        assert_eq!(v.status, Status::Completed);
        assert_eq!(s.seen.lock().unwrap().len(), 1);
        assert_eq!(
            std::fs::read_to_string(r.0.join("README.md")).unwrap(),
            "outdated"
        );
        assert!(
            v.events
                .iter()
                .any(|event| event.kind == "task_verification" && event.data["satisfied"] == false)
        );
    }
}
#[tokio::test]
async fn patch_turn_then_final_reply_completes_in_the_same_task() {
    let (v, s, r) = drive(
        "Update README.md",
        vec![
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
    let seen = s.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let messages = &seen[1].0;
    let call_id = &messages[messages.len() - 2]["tool_calls"][0]["id"];
    assert!(call_id.is_string());
    assert_eq!(&messages.last().unwrap()["tool_call_id"], call_id);
    assert_eq!(
        v.events.iter().filter(|e| e.kind == "file_changes").count(),
        1
    );
}
#[tokio::test]
async fn combined_request_audit_distinguishes_edit_and_inspection() {
    let (v, script, _) = drive(
        "Inspect this repository and update README.md",
        vec![patch("README.md"), text("Updated README.md")],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(v.status, Status::Completed);
    assert!(
        v.events
            .iter()
            .any(|event| event.kind == "task_verification" && event.data["satisfied"] == false)
    );
    assert_eq!(script.seen.lock().unwrap().len(), 2);
    let (v, script, _) = drive(
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
    assert_eq!(script.seen.lock().unwrap().len(), 3);
    assert!(
        v.events
            .iter()
            .any(|event| event.kind == "task_verification" && event.data["satisfied"] == true)
    );
}
#[tokio::test]
async fn a_different_successful_patch_cannot_satisfy_readme_update() {
    let (v, script, r) = drive(
        "Update README.md",
        vec![
            tool(
                "apply_patch",
                json!({"path":"other.md","old_text":"","new_text":"other"}),
            ),
            text("Done"),
        ],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(v.status, Status::Completed);
    assert!(
        v.events
            .iter()
            .any(|event| event.kind == "task_verification" && event.data["satisfied"] == false)
    );
    assert_eq!(script.seen.lock().unwrap().len(), 2);
    assert_eq!(
        std::fs::read_to_string(r.0.join("README.md")).unwrap(),
        "outdated"
    );
}
#[tokio::test]
async fn empty_reply_gets_one_recovery_and_preserves_the_approved_edit() {
    let (v, s, _) = drive(
        "Update README.md",
        vec![patch("README.md"), text(""), text("")],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(v.status, Status::Incomplete);
    assert!(questions(&v).is_empty());
    assert_eq!(s.seen.lock().unwrap().len(), 3);
    assert_eq!(
        v.events.iter().filter(|e| e.kind == "file_changes").count(),
        1
    );
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
async fn a_read_does_not_force_the_model_into_finalization() {
    let (v, s, _) = drive(
        "Read src/lib.rs. Do not edit files.",
        vec![
            tool("read_file", json!({"path":"src/lib.rs"})),
            tool("read_file", json!({"path":"README.md"})),
            text("src/lib.rs defines value returning 1."),
        ],
        Decision::Deny,
    )
    .await;
    assert_eq!(v.status, Status::Completed);
    let seen = s.seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert!(seen.iter().all(|(_, schemas)| *schemas > 0));
    assert_eq!(read_results(&v).len(), 2);
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

fn module_fixture() -> Fixture {
    let root = Fixture::new();
    std::fs::create_dir_all(root.0.join("src/tools/agent")).unwrap();
    std::fs::write(
        root.0.join("src/tools/agent/mod.rs"),
        "pub fn register() { /* real agent drawer */ }\n",
    )
    .unwrap();
    root
}
fn read_results(view: &TaskView) -> Vec<Value> {
    view.transcript
        .iter()
        .filter(|m| m["role"] == "tool" && m["tool_name"] == "read_file")
        .map(|m| serde_json::from_str(m["content"].as_str().unwrap()).unwrap())
        .collect()
}
#[tokio::test]
async fn unverified_read_automatically_discovers_exact_file_and_finishes_once() {
    let root = module_fixture();
    assert!(
        Workspace::new(&root.0)
            .unwrap()
            .require_reference("src/tools/agent/mod.rs")
            .is_err()
    );
    let (view, script, _) = drive_in(
        root,
        "Read mod.rs and explain it",
        vec![
            tool("read_file", json!({"path":"./src/tools/agent/mod.rs"})),
            text("The agent drawer module defines register()."),
        ],
        Decision::Deny,
    )
    .await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(script.seen.lock().unwrap().len(), 2);
    assert_eq!(
        view.events
            .iter()
            .filter(|e| e.kind == "tool_recovery")
            .count(),
        1
    );
    assert_eq!(
        view.events
            .iter()
            .filter(|e| e.kind == "task_continued" || e.kind == "finalization_requested")
            .count(),
        0
    );
    let results = read_results(&view);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["ok"], true);
    assert_eq!(results[0]["path"], "src/tools/agent/mod.rs");
    assert!(
        results[0]["content"]
            .as_str()
            .unwrap()
            .contains("real agent drawer")
    );
    assert_eq!(results[0]["recovery"]["discoveredWith"], "list_directory");
    assert!(
        view.events
            .iter()
            .any(|e| e.kind == "tool_completed" && e.data["tool"] == "list_directory")
    );
    assert!(!view.events.iter().any(|e| e.kind == "approval_required"));
}
#[tokio::test]
async fn successful_nested_read_is_retained_after_other_tools_and_has_no_completion_retry() {
    let (view, script, _) = drive_in(
        module_fixture(),
        "Read mod.rs in src/tools/agent/ and explain it",
        vec![
            tool("list_directory", json!({"path":"src/tools/agent"})),
            tool("read_file", json!({"path":"src/tools/agent/./mod.rs"})),
            tool("read_file", json!({"path":"README.md"})),
            tool("list_directory", json!({"path":"src"})),
            text("register() defines registration for the agent drawer."),
        ],
        Decision::Deny,
    )
    .await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(script.seen.lock().unwrap().len(), 5);
    assert!(
        !view
            .events
            .iter()
            .any(|e| e.kind == "tool_recovery" || e.kind == "task_continued")
    );
    assert!(
        view.events
            .iter()
            .any(|e| e.kind == "task_verification" && e.data["satisfied"] == true)
    );
}
#[tokio::test]
async fn requested_read_evidence_survives_model_context_compaction() {
    let root = module_fixture();
    let mut turns = vec![tool("read_file", json!({"path":"src/tools/agent/mod.rs"}))];
    for n in 0..4 {
        let name = format!("large{n}.txt");
        std::fs::write(root.0.join(&name), "source text\n".repeat(1000)).unwrap();
        turns.push(tool("read_file", json!({"path":name})));
    }
    turns.push(text(
        "The module defines the agent drawer registration function.",
    ));
    let (view, script, _) = drive_with_capacity(
        root,
        "Read mod.rs and explain it",
        turns,
        Decision::Deny,
        Some(8192),
    )
    .await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(script.seen.lock().unwrap().len(), 6);
    assert!(view.events.iter().any(|e| e.kind == "context_compacted"));
    assert!(
        read_results(&view)
            .iter()
            .any(|v| v["path"] == "src/tools/agent/mod.rs" && v["ok"] == true)
    );
    assert!(!view.events.iter().any(|e| e.kind == "task_continued"));
}
#[tokio::test]
async fn missing_exact_file_is_a_clear_blocker_without_summary_retries() {
    let (view, script, _) = drive_in(
        module_fixture(),
        "Read src/tools/agent/missing.rs",
        vec![
            tool("read_file", json!({"path":"src/tools/agent/missing.rs"})),
            text("The requested file could not be found."),
        ],
        Decision::Deny,
    )
    .await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(script.seen.lock().unwrap().len(), 2);
    assert!(
        read_results(&view)[0]["error"]
            .as_str()
            .unwrap()
            .contains("Could not confirm the exact requested file")
    );
    assert_eq!(
        view.events
            .iter()
            .filter(|e| e.kind == "tool_recovery")
            .count(),
        1
    );
    assert!(!view.events.iter().any(|e| e.kind == "task_continued"));
    assert!(read_results(&view).iter().all(|v| v["ok"] == false));
}
#[tokio::test]
async fn repeated_unverified_read_attempts_do_not_repeat_automatic_discovery() {
    let (view, _, _) = drive_in(
        module_fixture(),
        "Read src/tools/agent/missing.rs",
        vec![
            tool("read_file", json!({"path":"src/tools/agent/missing.rs"})),
            tool(
                "read_file",
                json!({"path":"./src/tools/agent/./missing.rs"}),
            ),
            text("The requested file is missing."),
        ],
        Decision::Deny,
    )
    .await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(
        view.events
            .iter()
            .filter(|e| e.kind == "tool_recovery")
            .count(),
        1
    );
    assert_eq!(
        view.events
            .iter()
            .filter(|e| e.kind == "tool_requested" && e.data["recoveryOf"].is_string())
            .count(),
        2
    );
}
#[tokio::test]
async fn prohibited_reads_never_trigger_discovery_or_claim_success() {
    for path in [".git/secret.txt", "../outside.txt", "/etc/passwd"] {
        let (view, script, _) = drive(
            &format!("Read the file {path}"),
            vec![
                tool("read_file", json!({"path":path})),
                text("That path is outside the allowed file scope."),
            ],
            Decision::Deny,
        )
        .await;
        assert_eq!(view.status, Status::Completed, "{path}: {:?}", view.result);
        assert_eq!(script.seen.lock().unwrap().len(), 2);
        assert!(!view.events.iter().any(|e| e.kind == "tool_recovery"
            || (e.kind == "tool_started" && e.data["tool"] != "request_user_input")
            || e.kind == "task_continued"));
        assert!(read_results(&view).iter().all(|v| v["ok"] == false));
    }
}
#[tokio::test]
async fn recovery_preserves_read_excerpt_arguments() {
    let (view, _, _) = drive_in(
        module_fixture(),
        "Read src/tools/agent/mod.rs bytes 4..8",
        vec![
            tool(
                "read_file",
                json!({"path":"src/tools/agent/mod.rs","offset":4,"length":4}),
            ),
            text("The requested excerpt is fn r."),
        ],
        Decision::Deny,
    )
    .await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    let read = &read_results(&view)[0];
    assert_eq!(read["offset"], 4);
    assert_eq!(read["content"], "fn r");
    assert_eq!(read["nextOffset"], 8);
}

struct ApprovedDiscovery;
impl Tool for ApprovedDiscovery {
    fn schema(&self) -> Value {
        json!({"type":"function","function":{"name":"list_directory"}})
    }
    fn prepare(&self, _: &Workspace, args: Value, _: &Limits) -> Result<Prepared> {
        Ok(Prepared {
            action: args.clone(),
            payload: args,
            approval: true,
        })
    }
    fn execute<'a>(
        &'a self,
        _: &'a Workspace,
        _: &'a Prepared,
        _: &'a Limits,
        _: tokio::sync::watch::Receiver<bool>,
        _: &'a mut (dyn FnMut(Value) + Send),
    ) -> FutureResult<'a, Value> {
        Box::pin(async { panic!("Denied discovery must never execute") })
    }
}
#[tokio::test]
async fn recovery_obeys_registered_discovery_approval_and_never_bypasses_denial() {
    let root = module_fixture();
    let agent = Arc::new(Agent::default());
    let mut registry = Registry::default();
    registry.add("list_directory", Arc::new(ApprovedDiscovery));
    let script = Script::new(vec![tool(
        "read_file",
        json!({"path":"src/tools/agent/mod.rs"}),
    )]);
    let id = agent
        .start(
            "test",
            "Read src/tools/agent/mod.rs",
            &root.0,
            Limits::default(),
            script.clone(),
            registry,
        )
        .unwrap();
    let view = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let v = agent.snapshot().unwrap();
            if let Some(approval) = &v.approval {
                agent
                    .decide("test", &id, &approval.id, Decision::Deny)
                    .unwrap();
            }
            if !v.status.active() {
                break v;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(view.status, Status::Denied, "{:?}", view.result);
    assert_eq!(script.seen.lock().unwrap().len(), 1);
    assert!(
        !view
            .events
            .iter()
            .any(|e| e.kind == "tool_requested" && e.data["tool"] == "search_files")
    );
    assert!(read_results(&view).iter().all(|v| v["ok"] == false));
}
#[cfg(unix)]
#[tokio::test]
async fn automatic_discovery_cannot_follow_a_file_symlink_outside_workspace() {
    let root = module_fixture();
    let outside = Fixture::new();
    std::os::unix::fs::symlink(
        outside.0.join("README.md"),
        root.0.join("src/tools/agent/escape.rs"),
    )
    .unwrap();
    let (view, script, _) = drive_in(
        root,
        "Read src/tools/agent/escape.rs",
        vec![
            tool("read_file", json!({"path":"src/tools/agent/escape.rs"})),
            text("The file could not be verified inside the workspace."),
        ],
        Decision::Deny,
    )
    .await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(script.seen.lock().unwrap().len(), 2);
    assert!(read_results(&view).iter().all(|v| v["ok"] == false));
    assert!(
        !view.transcript.iter().any(
            |m| m["role"] == "tool" && m["content"].as_str().unwrap_or("").contains("outdated")
        )
    );
}

struct TerminalScript {
    script: Arc<Script>,
    schemas: Mutex<Vec<Vec<Value>>>,
}
impl Inference for TerminalScript {
    fn generate<'a>(&'a self, m: &'a [Value], t: &'a [Value]) -> FutureResult<'a, Turn> {
        Box::pin(async move {
            self.schemas.lock().unwrap().push(t.to_vec());
            if let Some(last) = m.last().filter(|m| m["role"] == "tool")
                && let Ok(result) =
                    serde_json::from_str::<Value>(last["content"].as_str().unwrap_or(""))
                && result["running"] == true
            {
                return parse_turn(tool(
                    "write_stdin",
                    json!({"session_id":result["sessionId"],"yield_time_ms":1000}),
                ));
            }
            self.script.generate(m, t).await
        })
    }
}
#[cfg(target_os = "macos")]
#[tokio::test]
async fn malformed_command_arguments_get_schema_feedback_then_one_approved_real_execution() {
    let root = Fixture::new();
    std::fs::write(
        root.0.join("Cargo.toml"),
        "[package]\nname=\"argument_recovery_fixture\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
    )
    .unwrap();
    let source = "#[test] fn smoke(){assert_eq!(2+2,4);}\n";
    std::fs::write(root.0.join("src/lib.rs"), source).unwrap();
    let bad = json!({"arguments":"[\"cargo\", \"test\"]","directory":root.0});
    let script = Script::new(vec![
        tool(
            "exec_command",
            json!({"command":format!("cd {} && cargo test",root.0.display())}),
        ),
        tool("exec_command", bad.clone()),
        tool("exec_command", bad),
        tool(
            "exec_command",
            json!({"executable":"cargo","arguments":["test"],"directory":"."}),
        ),
        text("cargo test exited with code 0; the test passed."),
    ]);
    let model = Arc::new(TerminalScript {
        script: script.clone(),
        schemas: Mutex::new(vec![]),
    });
    let agent = Arc::new(Agent::default());
    let id = agent
        .start(
            "test",
            "Run cargo test; do not modify any source files",
            &root.0,
            Limits::default(),
            model.clone(),
            Registry::default(),
        )
        .unwrap();
    let mut approvals = std::collections::HashSet::new();
    let view = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let v = agent.snapshot().unwrap();
            if let Some(a) = &v.approval
                && approvals.insert(a.id.clone())
            {
                agent
                    .decide("test", &id, &a.id, Decision::AllowOnce)
                    .unwrap();
            }
            if !v.status.active() {
                break v;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(approvals.len(), 1);
    assert_eq!(script.seen.lock().unwrap().len(), 5);
    assert_eq!(
        view.events
            .iter()
            .filter(|e| e.kind == "tool_started" && e.data["tool"] == "exec_command")
            .count(),
        1
    );
    let errors: Vec<_> = view
        .events
        .iter()
        .filter(|e| e.kind == "tool_failed")
        .map(|e| &e.data["result"])
        .collect();
    assert_eq!(errors.len(), 3);
    for e in &errors {
        assert_eq!(e["code"], "invalid_arguments");
        assert_eq!(e["executed"], false);
        assert!(e["hint"].is_string());
        assert_eq!(
            e["expectedSchema"]["properties"]["arguments"]["type"],
            "array"
        );
        assert_eq!(
            e["example"],
            json!({"executable":"cargo","arguments":["test"],"directory":"."})
        );
    }
    assert_eq!(errors[2]["blockedIdenticalRetry"], true);
    let schemas = model.schemas.lock().unwrap();
    assert!(
        schemas[0]
            .iter()
            .any(|s| s["function"]["name"] == "tool_search")
    );
    assert!(
        schemas[0]
            .iter()
            .any(|s| s["function"]["name"] == "exec_command")
    );
    assert!(
        schemas
            .iter()
            .skip(1)
            .any(|ss| ss.iter().any(|s| s["function"]["name"] == "exec_command"))
    );
    assert!(
        view.events
            .iter()
            .any(|e| e.kind == "tool_completed" && e.data["result"]["exitCode"] == 0)
    );
    assert_eq!(
        std::fs::read_to_string(root.0.join("src/lib.rs")).unwrap(),
        source
    );
}
#[tokio::test]
async fn corrected_arguments_still_require_approval_and_denial_stops_execution() {
    let (view, script, _) = drive(
        "Run cargo test",
        vec![
            tool("exec_command", json!({"command":"cargo test"})),
            tool(
                "exec_command",
                json!({"executable":"cargo","arguments":["test"],"directory":"."}),
            ),
        ],
        Decision::Deny,
    )
    .await;
    assert_eq!(view.status, Status::Denied, "{:?}", view.result);
    assert_eq!(script.seen.lock().unwrap().len(), 2);
    assert_eq!(
        view.events
            .iter()
            .filter(|e| e.kind == "approval_required")
            .count(),
        1
    );
    assert!(!view.events.iter().any(|e| e.kind == "tool_started"));
}
#[tokio::test]
async fn essential_patch_format_error_keeps_exact_schema_and_preserves_valid_retry() {
    let (view,script,root)=drive("Update README.md",vec![
        tool("apply_patch",json!({"file":"README.md","old_text":"outdated","new_text":"accurate implementation"})),
        patch("README.md"),text("README.md is updated."),
    ],Decision::AllowOnce).await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    let seen = script.seen.lock().unwrap();
    assert_eq!(
        seen[1].1, seen[0].1,
        "Patch schema is available before the first call"
    );
    assert!(
        script.schemas.lock().unwrap()[0]
            .iter()
            .any(|s| s["function"]["name"] == "apply_patch")
    );
    let error = view
        .events
        .iter()
        .find(|e| e.kind == "tool_failed")
        .unwrap();
    assert_eq!(error.data["result"]["code"], "invalid_arguments");
    assert!(
        error.data["result"]["expectedSchema"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("path"))
    );
    assert_eq!(
        std::fs::read_to_string(root.0.join("README.md")).unwrap(),
        "accurate implementation"
    );
}

fn empty_workspace() -> Fixture {
    let root = Fixture::new();
    std::fs::remove_file(root.0.join("README.md")).unwrap();
    std::fs::remove_dir_all(root.0.join("src")).unwrap();
    root
}
fn questions(view: &TaskView) -> Vec<Value> {
    view.transcript
        .iter()
        .filter(|m| m["role"] == "tool" && m["tool_name"] == "request_user_input")
        .map(|m| serde_json::from_str(m["content"].as_str().unwrap()).unwrap())
        .collect()
}
#[tokio::test]
async fn clear_empty_readme_discovers_patch_schema_and_requires_one_approval() {
    let task = "Create README.md; do not populate it or do anything else.";
    let (view, script, root) = drive_in(
        empty_workspace(),
        task,
        vec![
            tool("tool_search", json!({"query":"apply_patch"})),
            tool(
                "apply_patch",
                json!({"path":"README.md","old_text":"","new_text":""}),
            ),
            text("Created an empty README.md."),
        ],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(
        std::fs::metadata(root.0.join("README.md")).unwrap().len(),
        0
    );
    let schemas = script.schemas.lock().unwrap();
    assert!(
        schemas[0]
            .iter()
            .any(|s| s["function"]["name"] == "tool_search")
    );
    assert!(
        schemas[0]
            .iter()
            .any(|s| s["function"]["name"] == "apply_patch")
    );
    let patch = schemas[1]
        .iter()
        .find(|s| s["function"]["name"] == "apply_patch")
        .unwrap();
    assert_eq!(
        patch["function"]["parameters"]["additionalProperties"],
        false
    );
    assert!(
        patch["function"]["parameters"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("new_text"))
    );
    assert!(questions(&view).is_empty());
    assert_eq!(
        view.events
            .iter()
            .filter(|e| e.kind == "approval_required")
            .count(),
        1
    );
    assert!(!view.events.iter().any(|e| e.kind == "tool_requested"
        && e.data["tool"] != "apply_patch"
        && e.data["tool"] != "tool_search"));
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 1);
}
#[tokio::test]
async fn an_empty_workspace_does_not_imply_the_requested_file_must_be_empty() {
    let (view, _, root) = drive_in(
        empty_workspace(),
        "Create README.md in this empty workspace",
        vec![
            tool(
                "apply_patch",
                json!({"path":"README.md","old_text":"","new_text":"# Playground\n"}),
            ),
            text("Created README.md."),
        ],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(
        std::fs::read_to_string(root.0.join("README.md")).unwrap(),
        "# Playground\n"
    );
}
#[tokio::test]
async fn an_empty_file_constraint_blocks_a_title_before_approval_and_allows_correction() {
    let (view, _, root) = drive_in(
        empty_workspace(),
        "Create an empty README.md; do not populate it",
        vec![
            tool(
                "apply_patch",
                json!({"path":"README.md","old_text":"","new_text":"# README\n"}),
            ),
            tool(
                "apply_patch",
                json!({"path":"README.md","old_text":"","new_text":""}),
            ),
            text("Created README.md with zero bytes."),
        ],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(
        std::fs::metadata(root.0.join("README.md")).unwrap().len(),
        0
    );
    assert_eq!(
        view.events
            .iter()
            .filter(|e| e.kind == "approval_required")
            .count(),
        1
    );
    assert_eq!(
        view.events
            .iter()
            .filter(|e| e.kind == "file_changes")
            .count(),
        1
    );
    let rejected = view
        .events
        .iter()
        .find(|e| e.kind == "tool_failed")
        .unwrap();
    assert!(
        rejected.data["result"]["error"]
            .as_str()
            .unwrap()
            .contains("must be empty")
    );
}
#[tokio::test]
async fn ambiguous_intent_can_ask_one_targeted_free_text_question_without_side_effects() {
    let (view, script, root) = drive_in(
        empty_workspace(),
        "Create a file",
        vec![tool(
            "request_user_input",
            json!({"question":"What filename and contents should I use?"}),
        )],
        Decision::Deny,
    )
    .await;
    assert_eq!(view.status, Status::WaitingForInput);
    assert_eq!(questions(&view).len(), 1);
    assert_eq!(questions(&view)[0]["needsUserInput"], true);
    assert_eq!(script.seen.lock().unwrap().len(), 1);
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0);
    assert!(!view.events.iter().any(|e| e.kind == "approval_required"));
    assert!(
        script.schemas.lock().unwrap()[0]
            .iter()
            .any(|s| s["function"]["name"] == "tool_search")
    );
}
#[tokio::test]
async fn repeated_failures_stop_and_keep_completed_work() {
    let missing = tool("read_file", json!({"path":"missing.rs"}));
    let (view, script, root) = drive(
        "Update README.md and read missing.rs",
        vec![
            patch("README.md"),
            missing.clone(),
            missing.clone(),
            json!({"role":"assistant","tool_calls":[
                {"id":"third-read","function":{"name":"read_file","arguments":{"path":"missing.rs"}}},
                {"id":"unexecuted-read","function":{"name":"read_file","arguments":{"path":"README.md"}}}
            ]}),
        ],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(view.status, Status::Incomplete, "{:?}", view.result);
    assert!(questions(&view).is_empty());
    assert_eq!(script.seen.lock().unwrap().len(), 4);
    assert_eq!(
        std::fs::read_to_string(root.0.join("README.md")).unwrap(),
        "accurate implementation"
    );
    assert_eq!(
        view.events
            .iter()
            .filter(|e| e.kind == "file_changes")
            .count(),
        1
    );
    assert!(view.result.unwrap().contains("repeated unsuccessful"));
    assert_eq!(
        view.events
            .iter()
            .filter(|e| e.kind == "tool_recovery")
            .count(),
        1
    );
    assert!(
        view.events
            .iter()
            .any(|event| event.kind == "tool_skipped" && event.data["callId"] == "unexecuted-read")
    );
}
#[tokio::test]
async fn repeated_empty_listings_stop_without_a_generic_question() {
    let listing = tool("list_directory", json!({"path":"."}));
    let (view, script, _) = drive_in(
        empty_workspace(),
        "Inspect this directory",
        vec![listing.clone(), listing.clone(), listing.clone(), listing],
        Decision::Deny,
    )
    .await;
    assert_eq!(view.status, Status::Incomplete, "{:?}", view.result);
    assert!(questions(&view).is_empty());
    assert_eq!(script.seen.lock().unwrap().len(), 4);
    let first = &script.seen.lock().unwrap()[1].0.last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_owned();
    let result: Value = serde_json::from_str(first).unwrap();
    assert_eq!(result["empty"], true);
    assert_eq!(result["includesHidden"], true);
    assert_eq!(result["truncated"], false);
    assert!(view.result.unwrap().contains("unchanged results"));
}
#[tokio::test]
async fn awareness_and_scope_are_persistent_after_compaction_without_duplicate_system_prompts() {
    let root = Fixture::new();
    let mut turns = vec![patch("README.md")];
    for n in 0..5 {
        let path = format!("finding-{n}.txt");
        std::fs::write(
            root.0.join(&path),
            format!("real finding {n}\n").repeat(1000),
        )
        .unwrap();
        turns.push(tool("read_file", json!({"path":path})));
    }
    turns.push(text(
        "README.md was updated and the written bytes verified.",
    ));
    let task = "Update README.md; do not run commands";
    let (view, script, _) =
        drive_with_capacity(root, task, turns, Decision::AllowOnce, Some(8192)).await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert!(view.events.iter().any(|e| e.kind == "context_compacted"));
    for (messages, _) in script.seen.lock().unwrap().iter() {
        assert_eq!(messages.iter().filter(|m| m["role"] == "system").count(), 1);
        let policy = messages[0]["content"].as_str().unwrap();
        assert!(policy.contains("AI agent inside Lorca"));
        assert!(policy.contains("Rust validates arguments"));
        assert!(policy.contains("Never silently substitute"));
        assert!(policy.contains("Unavailable tools:"));
    }
    let seen = script.seen.lock().unwrap();
    let last = &seen.last().unwrap().0;
    let checkpoint = last
        .iter()
        .find(|m| {
            m["content"]
                .as_str()
                .is_some_and(|s| s.starts_with("Working checkpoint"))
        })
        .unwrap();
    let data = checkpoint["content"].as_str().unwrap();
    assert!(data.contains(task));
    assert!(data.contains("apply_patch"));
    assert!(data.contains("requiresActionApproval"));
    let checkpoint: Value = serde_json::from_str(
        data.strip_prefix("Working checkpoint (historical data): ")
            .unwrap(),
    )
    .unwrap();
    assert!(
        checkpoint["verifiedState"]["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["tool"] == "apply_patch" && v["verified"] == true && v["bytes"] == 23)
    );
}

#[tokio::test]
async fn a_plain_model_question_is_returned_as_a_reply() {
    let (view, script, root) = drive_in(
        empty_workspace(),
        "Create a file",
        vec![text(
            "I'll create a file for you. What filename and contents should I use?",
        )],
        Decision::Deny,
    )
    .await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(script.seen.lock().unwrap().len(), 1);
    assert!(questions(&view).is_empty());
    assert!(
        view.result
            .unwrap()
            .ends_with("What filename and contents should I use?")
    );
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0);
    assert!(!view.events.iter().any(|e| e.kind == "task_continued"));
}
#[tokio::test]
async fn malformed_top_level_tool_json_gets_one_safe_retry_with_current_schemas() {
    let (view, script, _) = drive(
        "Read README.md",
        vec![
            tool("read_file", json!(["README.md"])),
            tool("read_file", json!({"path":"README.md"})),
            text("README.md contains outdated."),
        ],
        Decision::Deny,
    )
    .await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(read_results(&view).len(), 1);
    assert_eq!(
        view.events
            .iter()
            .filter(|e| e.kind == "model_call_invalid")
            .count(),
        1
    );
    assert!(
        script.seen.lock().unwrap()[1].0.last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("No tool executed")
    );
    let (view, script, _) = drive(
        "Read README.md",
        vec![
            tool("read_file", json!(["README.md"])),
            tool("read_file", json!(["README.md"])),
        ],
        Decision::Deny,
    )
    .await;
    assert_eq!(view.status, Status::Incomplete);
    assert!(questions(&view).is_empty());
    assert!(read_results(&view).is_empty());
    assert_eq!(script.seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn distinct_deep_read_paths_use_bounded_internal_discovery_without_model_bookkeeping() {
    let root = empty_workspace();
    std::fs::create_dir_all(root.0.join("src/tools/agent")).unwrap();
    let mut turns = Vec::new();
    for n in 0..4 {
        let path = format!("src/tools/agent/deep-{n}.rs");
        std::fs::write(root.0.join(&path), format!("// real module {n}\n")).unwrap();
        turns.push(tool("read_file", json!({"path":path})));
    }
    // Keep the reads first so they still exercise automatic discovery.
    turns.push(tool("list_directory", json!({"path":"src/tools/agent"})));
    turns.push(text(
        "The four actual modules contain their respective module numbers.",
    ));
    let (view, script, _) = drive_in(
        root,
        "Read the files in src/tools/agent/",
        turns,
        Decision::Deny,
    )
    .await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(script.seen.lock().unwrap().len(), 6);
    assert_eq!(read_results(&view).len(), 4);
    assert!(read_results(&view).iter().all(|r| r["ok"] == true));
    assert!(read_results(&view).iter().all(|r| r["truncated"] == false));
    let recovery: Vec<_> = view
        .events
        .iter()
        .filter(|event| event.kind == "tool_recovery")
        .collect();
    assert_eq!(recovery.len(), 1);
    let first_call = &recovery[0].data["callId"];
    assert!(
        view.events
            .iter()
            .any(|event| event.kind == "tool_completed"
                && event.data["recoveryOf"] == *first_call
                && event.data["tool"] == "list_directory")
    );
    let paths: Vec<_> = read_results(&view)
        .into_iter()
        .map(|r| r["path"].clone())
        .collect();
    assert_eq!(
        paths,
        (0..4)
            .map(|n| json!(format!("src/tools/agent/deep-{n}.rs")))
            .collect::<Vec<_>>()
    );
    assert!(questions(&view).is_empty());
    assert!(!view.events.iter().any(|e| e.kind == "tool_failed"));
}

#[tokio::test]
async fn safe_path_spelling_does_not_bypass_empty_constraint_or_lose_completion_evidence() {
    let (view, _, root) = drive_in(
        empty_workspace(),
        "Create an empty README.md",
        vec![
            tool(
                "apply_patch",
                json!({"path":"./README.md","old_text":"","new_text":"title"}),
            ),
            tool(
                "apply_patch",
                json!({"path":"./README.md","old_text":"","new_text":""}),
            ),
            text("Created the empty README.md."),
        ],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(
        std::fs::metadata(root.0.join("README.md")).unwrap().len(),
        0
    );
    assert_eq!(
        view.events
            .iter()
            .filter(|e| e.kind == "approval_required")
            .count(),
        1
    );
}
#[tokio::test]
async fn corrected_minor_errors_do_not_consume_a_stale_global_failure_budget() {
    let root = empty_workspace();
    let mut turns = Vec::new();
    for n in 0..6 {
        let path = format!("file-{n}.txt");
        std::fs::write(root.0.join(&path), "real content").unwrap();
        turns.push(tool("read_file", json!({"path":path,"unknown":true})));
        turns.push(tool("read_file", json!({"path":path})));
    }
    turns.push(tool("list_directory", json!({"path":"."})));
    turns.push(text("The six files contain real content."));
    let (view, _, _) = drive_in(root, "Read the workspace files", turns, Decision::Deny).await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(
        read_results(&view)
            .iter()
            .filter(|r| r["ok"] == true)
            .count(),
        6
    );
    assert!(
        read_results(&view)
            .iter()
            .filter(|r| r["ok"] == true)
            .all(|r| r["truncated"] == false)
    );
    let reads = read_results(&view);
    assert_eq!(reads.len(), 12);
    for (n, pair) in reads.as_chunks::<2>().0.iter().enumerate() {
        assert_eq!(pair[0]["ok"], false);
        assert_eq!(pair[0]["code"], "invalid_arguments");
        assert_eq!(pair[1]["ok"], true);
        assert_eq!(pair[1]["path"], format!("file-{n}.txt"));
    }
    assert!(
        !view
            .events
            .iter()
            .any(|event| event.kind == "approval_required")
    );
    assert!(questions(&view).is_empty());
}

fn working_plan(next: &str) -> Value {
    tool(
        "update_plan",
        json!({"steps":[{"step":"Choose a format","status":"in_progress"}],
        "checkpoint":{"approach":"Use a small targeted edit","decisions":["Keep existing layout"],"uncertainties":[],"nextAction":next}}),
    )
}
#[tokio::test]
async fn working_notes_never_prove_an_edit_and_planning_cannot_loop() {
    let (view, script, _root) = drive(
        "Update README.md",
        vec![
            tool("tool_search", json!({"query":"update_plan"})),
            working_plan("Edit README.md"),
            working_plan("Reconsider README.md"),
            text("I updated README.md."),
        ],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(view.status, Status::Incomplete);
    assert!(view.result.unwrap().contains("Working checkpoints"));
    assert_eq!(script.seen.lock().unwrap().len(), 3);
    assert!(!view.audit.iter().any(|e| e.kind == "file_changes"));
}
#[tokio::test]
async fn a_recommendation_waits_for_the_user_without_claiming_failure_or_approval() {
    let (view, script, _root) = drive("Update README.md", vec![
        tool("request_user_input", json!({"question":"Which format?","recommendation":"Markdown keeps it simple.","options":["Markdown","Plain text"]})),
        patch("README.md"),
    ], Decision::AllowOnce).await;
    assert_eq!(view.status, Status::WaitingForInput);
    assert!(view.result.unwrap().contains("Recommendation: Markdown"));
    assert_eq!(script.seen.lock().unwrap().len(), 1);
    assert!(!view.audit.iter().any(|e| e.kind == "file_changes"));
}
#[tokio::test]
async fn checkpoints_allow_a_useful_next_action_and_do_not_replace_verification() {
    let (view, _, root) = drive(
        "Update README.md",
        vec![
            tool("tool_search", json!({"query":"update_plan"})),
            working_plan("Edit README.md"),
            patch("README.md"),
            text("README.md updated and verified."),
        ],
        Decision::AllowOnce,
    )
    .await;
    assert_eq!(view.status, Status::Completed);
    assert_eq!(
        std::fs::read_to_string(root.0.join("README.md")).unwrap(),
        "accurate implementation"
    );
    assert!(view.audit.iter().any(|e| e.kind == "approval_required"));
}

#[tokio::test]
async fn missing_answer_recovery_keeps_tools_and_is_not_renewed_by_a_successful_read() {
    let (view, script, _) = drive(
        "Inspect this repository",
        vec![
            tool("read_file", json!({"path":"README.md"})),
            text(""),
            tool("read_file", json!({"path":"src/lib.rs"})),
            text(""),
        ],
        Decision::Deny,
    )
    .await;
    assert_eq!(view.status, Status::Incomplete);
    assert_eq!(script.seen.lock().unwrap().len(), 4);
    assert!(
        script
            .seen
            .lock()
            .unwrap()
            .iter()
            .all(|(_, tools)| *tools > 0)
    );
    assert_eq!(
        view.audit
            .iter()
            .filter(|event| event.kind == "public_reply_recovery")
            .count(),
        1
    );
    assert_eq!(read_results(&view).len(), 2);
    assert!(questions(&view).is_empty());
}
