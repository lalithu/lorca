use lorca::{agent::*, provider::agent::parse_turn};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("lorca-agent-test-{}", lorca::types::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Mock {
    turns: Mutex<VecDeque<Turn>>,
    seen: Mutex<Vec<Vec<Value>>>,
}
impl Mock {
    fn new(turns: Vec<Value>) -> Arc<Self> {
        Arc::new(Self {
            turns: Mutex::new(turns.into_iter().map(|v| parse_turn(v).unwrap()).collect()),
            seen: Mutex::new(vec![]),
        })
    }
}
impl Inference for Mock {
    fn generate<'a>(&'a self, messages: &'a [Value], _: &'a [Value]) -> FutureResult<'a, Turn> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(messages.to_vec());
            self.turns
                .lock()
                .unwrap()
                .pop_front()
                .ok_or("No mock turns".into())
        })
    }
}
fn calls(c: Vec<(&str, Value)>) -> Value {
    json!({"role":"assistant","content":"","tool_calls":c.into_iter().map(|(n,a)|json!({"function":{"name":n,"arguments":a}})).collect::<Vec<_>>()})
}
fn finish() -> Value {
    json!({"role":"assistant","content":"Verified the actual tool results."})
}
async fn wait(agent: &Agent, predicate: impl Fn(&TaskView) -> bool) -> TaskView {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Some(v) = agent.snapshot()
                && predicate(&v)
            {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("task stalled")
}
fn start(agent: &Arc<Agent>, root: &Path, mock: Arc<Mock>) -> String {
    agent
        .start(
            "test",
            "Perform the requested tool operations",
            root,
            Limits::default(),
            mock,
            Registry::default(),
        )
        .unwrap()
}
#[test]
fn parses_native_calls_and_rejects_text_or_invalid_arguments() {
    let turn=parse_turn(json!({"role":"assistant","content":"public","thinking":"private","tool_calls":[{"function":{"name":"read_file","arguments":"{\"path\":\"src/lib.rs\"}"}},{"id":"second","function":{"name":"list_directory","arguments":{"path":"."}}}]})).unwrap();
    assert_eq!(turn.calls.len(), 2);
    assert_eq!(turn.calls[1].id, "second");
    assert!(turn.message.get("thinking").is_none());
    assert_eq!(turn.message["tool_calls"][0]["id"], turn.calls[0].id);
    assert!(parse_turn(json!({"role":"assistant","tool_calls":[{"function":{"name":"read_file","arguments":[]}}]})).is_err());
    assert!(parse_turn(json!({"role":"tool","content":"bad"})).is_err());
    assert!(
        parse_turn(json!({"role":"assistant","content":"run_command cargo test"}))
            .unwrap()
            .calls
            .is_empty()
    );
}
#[tokio::test]
async fn multiple_calls_return_real_results_and_ordered_stable_events() {
    let root = Temp::new();
    std::fs::write(root.0.join("hello.rs"), "fn hello() {}\n").unwrap();
    let mock = Mock::new(vec![
        calls(vec![
            ("list_directory", json!({"path":"."})),
            ("read_file", json!({"path":"hello.rs"})),
        ]),
        finish(),
    ]);
    let agent = Arc::new(Agent::default());
    let id = start(&agent, &root.0, mock.clone());
    let view = wait(&agent, |v| !v.status.active()).await;
    assert_eq!(view.status, Status::Completed);
    let seen = mock.seen.lock().unwrap();
    let history = &seen[1];
    assert_eq!(history[2]["tool_calls"].as_array().unwrap().len(), 2);
    assert_eq!(history[3]["role"], "tool");
    assert_eq!(history[4]["tool_name"], "read_file");
    assert!(history[4]["content"].as_str().unwrap().contains("fn hello"));
    for (i, e) in view.events.iter().enumerate() {
        assert_eq!(e.sequence, i as u64 + 1);
        assert_eq!(e.task_id, id);
        assert_eq!(e.id, format!("{id}:{}", i + 1));
    }
    assert_eq!(view.events.first().unwrap().kind, "task_started");
    assert_eq!(view.events.last().unwrap().kind, "task_completed");
}
#[tokio::test]
async fn approval_is_exact_single_use_and_patch_is_verified() {
    let root = Temp::new();
    std::fs::write(root.0.join("a.rs"), "old\n").unwrap();
    let mock = Mock::new(vec![
        calls(vec![(
            "apply_patch",
            json!({"path":"a.rs","old_text":"old","new_text":"new"}),
        )]),
        finish(),
    ]);
    let agent = Arc::new(Agent::default());
    let id = start(&agent, &root.0, mock);
    let pending = wait(&agent, |v| v.approval.is_some())
        .await
        .approval
        .unwrap();
    assert!(
        pending.action["diff"]
            .as_str()
            .unwrap()
            .contains("-old\n+new")
    );
    assert_eq!(
        std::fs::read_to_string(root.0.join("a.rs")).unwrap(),
        "old\n"
    );
    assert!(
        agent
            .decide("other", &id, &pending.id, Decision::AllowOnce)
            .is_err()
    );
    assert!(
        agent
            .decide("test", &id, "wrong", Decision::AllowOnce)
            .is_err()
    );
    agent
        .decide("test", &id, &pending.id, Decision::AllowOnce)
        .unwrap();
    assert!(
        agent
            .decide("test", &id, &pending.id, Decision::AllowOnce)
            .is_err()
    );
    let view = wait(&agent, |v| !v.status.active()).await;
    assert_eq!(view.status, Status::Completed);
    assert_eq!(
        std::fs::read_to_string(root.0.join("a.rs")).unwrap(),
        "new\n"
    );
    assert!(
        view.events
            .iter()
            .any(|e| e.kind == "file_changes" && e.data["verified"] == true)
    );
}
#[tokio::test]
async fn denied_and_cancelled_actions_never_write_or_report_success() {
    for decision in [Decision::Deny, Decision::Cancel] {
        let root = Temp::new();
        let mock = Mock::new(vec![
            calls(vec![(
                "apply_patch",
                json!({"path":"new.txt","old_text":"","new_text":"unsafe"}),
            )]),
            finish(),
        ]);
        let agent = Arc::new(Agent::default());
        let id = start(&agent, &root.0, mock);
        let a = wait(&agent, |v| v.approval.is_some())
            .await
            .approval
            .unwrap();
        agent.decide("test", &id, &a.id, decision).unwrap();
        let view = wait(&agent, |v| !v.status.active()).await;
        assert!(!root.0.join("new.txt").exists());
        assert_ne!(view.status, Status::Completed);
        assert!(!view.events.iter().any(|e| e.kind == "tool_started"));
    }
}
#[tokio::test]
async fn stale_patch_and_invalid_multi_match_leave_files_intact() {
    let root = Temp::new();
    let file = root.0.join("a");
    std::fs::write(&file, "before").unwrap();
    let agent = Arc::new(Agent::default());
    let mock = Mock::new(vec![
        calls(vec![(
            "apply_patch",
            json!({"path":"a","old_text":"before","new_text":"after"}),
        )]),
        finish(),
    ]);
    let id = start(&agent, &root.0, mock);
    let a = wait(&agent, |v| v.approval.is_some())
        .await
        .approval
        .unwrap();
    std::fs::write(&file, "human edit").unwrap();
    agent
        .decide("test", &id, &a.id, Decision::AllowOnce)
        .unwrap();
    assert_eq!(
        wait(&agent, |v| !v.status.active()).await.status,
        Status::Failed
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "human edit");
    let workspace = Workspace::new(&root.0).unwrap();
    let registry = Registry::default();
    std::fs::write(&file, "aa").unwrap();
    assert!(
        registry
            .get("apply_patch")
            .unwrap()
            .prepare(
                &workspace,
                json!({"path":"a","old_text":"a","new_text":"b"}),
                &Limits::default()
            )
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "aa");
}
#[cfg(unix)]
#[test]
fn workspace_rejects_traversal_outside_symlinks_and_write_symlinks() {
    use std::os::unix::fs::symlink;
    let root = Temp::new();
    let outside = Temp::new();
    std::fs::write(outside.0.join("secret"), "secret").unwrap();
    symlink(&outside.0, root.0.join("escape")).unwrap();
    std::fs::write(root.0.join("safe"), "safe").unwrap();
    symlink("safe", root.0.join("internal")).unwrap();
    let w = Workspace::new(&root.0).unwrap();
    for path in ["../secret", "/etc/passwd", "escape/secret", "missing", "."] {
        assert!(w.read(path).is_err(), "{path}");
    }
    assert_eq!(w.read("internal").unwrap(), "safe");
    assert!(w.patch_source("internal").is_err());
    assert!(w.directory("escape").is_err());
    assert!(w.replace(".git/config", &None, "bad").is_err());
    assert!(
        Registry::default()
            .get("read_file")
            .unwrap()
            .prepare(&w, json!({"path":"safe","extra":true}), &Limits::default())
            .is_err()
    );
}
#[tokio::test]
async fn repeated_failures_limit_and_single_active_task_are_enforced() {
    let root = Temp::new();
    let turn = calls(vec![("read_file", json!({"path":"missing"}))]);
    let mock = Mock::new(vec![turn.clone(), turn.clone(), turn]);
    let agent = Arc::new(Agent::default());
    start(&agent, &root.0, mock.clone());
    assert!(
        agent
            .start(
                "other",
                "task",
                &root.0,
                Limits::default(),
                mock.clone(),
                Registry::default()
            )
            .is_err()
    );
    let view = wait(&agent, |v| !v.status.active()).await;
    assert_eq!(view.status, Status::WaitingForInput);
    assert!(view.result.unwrap().contains("repeated unsuccessful"));
    assert_eq!(mock.seen.lock().unwrap().len(), 3);
    let mock = Mock::new(vec![calls(vec![("list_directory", json!({"path":"."}))])]);
    agent
        .start(
            "test",
            "task",
            &root.0,
            Limits {
                iterations: 1,
                ..Default::default()
            },
            mock,
            Registry::default(),
        )
        .unwrap();
    assert!(
        wait(&agent, |v| !v.status.active())
            .await
            .result
            .unwrap()
            .contains("Iteration limit")
    );
}
struct Hanging;
impl Inference for Hanging {
    fn generate<'a>(&'a self, _: &'a [Value], _: &'a [Value]) -> FutureResult<'a, Turn> {
        Box::pin(std::future::pending())
    }
}
#[tokio::test]
async fn model_timeout_and_cancellation_leave_no_active_task() {
    let root = Temp::new();
    let agent = Arc::new(Agent::default());
    let limits = Limits {
        model_seconds: 1,
        ..Default::default()
    };
    agent
        .start(
            "test",
            "task",
            &root.0,
            limits,
            Arc::new(Hanging),
            Registry::default(),
        )
        .unwrap();
    assert!(
        wait(&agent, |v| !v.status.active())
            .await
            .result
            .unwrap()
            .contains("timed out")
    );
    let id = agent
        .start(
            "test",
            "task",
            &root.0,
            Limits::default(),
            Arc::new(Hanging),
            Registry::default(),
        )
        .unwrap();
    assert!(agent.cancel("other", &id).is_err());
    agent.cancel("test", &id).unwrap();
    assert_eq!(
        wait(&agent, |v| !v.status.active()).await.status,
        Status::Cancelled
    );
}
#[tokio::test]
async fn native_registry_rejects_unrestricted_commands_and_arguments() {
    let root = Temp::new();
    let w = Workspace::new(&root.0).unwrap();
    let tool = Registry::default().get("run_command").unwrap();
    for args in [
        json!({"executable":"sh","arguments":["-c","echo danger"]}),
        json!({"executable":"cargo","arguments":["test","--config","x=y"]}),
        json!({"executable":"sudo","arguments":[]}),
        json!({"executable":"cargo","arguments":["clean"]}),
        json!({"executable":"cargo","arguments":["test"],"directory":"../"}),
    ] {
        assert!(tool.prepare(&w, args, &Limits::default()).is_err());
    }
}

#[tokio::test]
async fn a_fresh_approved_patch_can_recover_from_a_stale_proposal() {
    let root = Temp::new();
    std::fs::write(root.0.join("a"), "original").unwrap();
    let mock = Mock::new(vec![
        calls(vec![(
            "apply_patch",
            json!({"path":"a","old_text":"original","new_text":"first"}),
        )]),
        calls(vec![(
            "apply_patch",
            json!({"path":"a","old_text":"human edit","new_text":"verified replacement"}),
        )]),
        finish(),
    ]);
    let agent = Arc::new(Agent::default());
    let id = start(&agent, &root.0, mock);
    let first = wait(&agent, |v| v.approval.is_some())
        .await
        .approval
        .unwrap();
    std::fs::write(root.0.join("a"), "human edit").unwrap();
    agent
        .decide("test", &id, &first.id, Decision::AllowOnce)
        .unwrap();
    let second = wait(&agent, |v| {
        v.approval.as_ref().is_some_and(|a| a.id != first.id)
    })
    .await
    .approval
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(root.0.join("a")).unwrap(),
        "human edit"
    );
    agent
        .decide("test", &id, &second.id, Decision::AllowOnce)
        .unwrap();
    assert_eq!(
        wait(&agent, |v| !v.status.active()).await.status,
        Status::Completed
    );
    assert_eq!(
        std::fs::read_to_string(root.0.join("a")).unwrap(),
        "verified replacement"
    );
}

#[tokio::test]
async fn text_only_interactions_complete_with_and_without_workspace() {
    for workspace in [false, true] {
        for text in ["Hi!", "Rust ownership gives each value an owner."] {
            let temp = Temp::new();
            let agent = Arc::new(Agent::default());
            let model = Mock::new(vec![json!({"role":"assistant","content":text})]);
            agent
                .submit(
                    Submission {
                        owner: "test".into(),
                        model: None,
                        task: "hi".into(),
                        workspace: if workspace {
                            temp.0.clone()
                        } else {
                            Default::default()
                        },
                        limits: Limits::default(),
                        conversation_id: None,
                        request_id: None,
                        history: vec![],
                    },
                    model.clone(),
                    if workspace {
                        Registry::default()
                    } else {
                        Registry::empty()
                    },
                )
                .unwrap();
            let done = wait(&agent, |v| !v.status.active()).await;
            assert_eq!(done.status, Status::Completed);
            assert_eq!(done.result.as_deref(), Some(text));
            assert!(!done.events.iter().any(|e| e.kind == "tool_requested"));
            assert_eq!(model.seen.lock().unwrap().len(), 1);
        }
    }
}
#[tokio::test]
async fn a_text_claim_does_not_replace_verified_execution() {
    let root = Temp::new();
    let agent = Arc::new(Agent::default());
    start(
        &agent,
        &root.0,
        Mock::new(vec![
            json!({"role":"assistant","content":"I changed the file and tests passed."}),
        ]),
    );
    let done = wait(&agent, |v| !v.status.active()).await;
    assert_eq!(done.status, Status::Failed);
    assert!(done.result.unwrap().contains("Unverified"));
}
#[tokio::test]
async fn discovery_resolves_guessed_paths_and_revalidates_references() {
    let root = Temp::new();
    std::fs::create_dir(root.0.join("src")).unwrap();
    std::fs::create_dir_all(root.0.join("src/deeper/core")).unwrap();
    std::fs::write(root.0.join("src/deeper/core/mod.rs"), "pub fn actual() {}").unwrap();
    let model = Mock::new(vec![
        calls(vec![("read_file", json!({"path":"src/core.rs"}))]),
        calls(vec![(
            "search_files",
            json!({"query":"mod.rs","path":"src"}),
        )]),
        calls(vec![(
            "read_file",
            json!({"path":"src/deeper/core/mod.rs"}),
        )]),
        finish(),
    ]);
    let agent = Arc::new(Agent::default());
    start(&agent, &root.0, model.clone());
    let done = wait(&agent, |v| !v.status.active()).await;
    assert_eq!(done.status, Status::Completed);
    assert!(done.events.iter().any(|e| {
        e.kind == "tool_failed"
            && e.data["result"]["error"]
                .as_str()
                .unwrap_or("")
                .contains("Unverified file")
    }));
    assert!(model.seen.lock().unwrap()[3].iter().any(|m| {
        m["content"]
            .as_str()
            .unwrap_or("")
            .contains("pub fn actual")
    }));
    let w = Workspace::new(&root.0).unwrap();
    assert!(w.require_reference("src/deeper/core/mod.rs").is_err());
    w.remember("src/deeper/core/mod.rs").unwrap();
    assert!(w.require_reference("src/deeper/core/mod.rs").is_ok());
    std::fs::remove_file(root.0.join("src/deeper/core/mod.rs")).unwrap();
    assert!(w.require_reference("src/deeper/core/mod.rs").is_err());
}

struct LowContextMock {
    model: Arc<Mock>,
}
impl Inference for LowContextMock {
    fn capacity(&self) -> FutureResult<'_, Option<u64>> {
        Box::pin(async { Ok(Some(12_000)) })
    }
    fn generate<'a>(&'a self, messages: &'a [Value], tools: &'a [Value]) -> FutureResult<'a, Turn> {
        self.model.generate(messages, tools)
    }
}
#[tokio::test]
async fn long_task_compacts_repeatedly_and_never_repeats_an_approved_write() {
    let root = Temp::new();
    std::fs::write(
        root.0.join("large.rs"),
        "verified repository finding\n".repeat(600),
    )
    .unwrap();
    let patch = calls(vec![(
        "apply_patch",
        json!({"path":"created.rs","old_text":"","new_text":"verified once"}),
    )]);
    let mut turns = vec![patch.clone()];
    for n in 0..22 {
        let path = format!("large-{n}.rs");
        std::fs::copy(root.0.join("large.rs"), root.0.join(&path)).unwrap();
        turns.push(calls(vec![("read_file", json!({"path":path}))]));
    }
    turns.push(patch);
    turns.push(finish());
    let model = Mock::new(turns);
    let agent = Arc::new(Agent::default());
    let id = agent
        .start(
            "test",
            "Create the file, inspect findings, preserve my goal",
            &root.0,
            Limits {
                iterations: 30,
                ..Default::default()
            },
            Arc::new(LowContextMock {
                model: model.clone(),
            }),
            Registry::default(),
        )
        .unwrap();
    let approval = wait(&agent, |v| v.approval.is_some())
        .await
        .approval
        .unwrap();
    agent
        .decide("test", &id, &approval.id, Decision::AllowOnce)
        .unwrap();
    let done = wait(&agent, |v| !v.status.active()).await;
    assert_eq!(done.status, Status::Completed, "{:?}", done.result);
    let compactions = done
        .events
        .iter()
        .filter(|e| e.kind == "context_compacted")
        .count();
    assert!(compactions >= 2, "compactions: {compactions}");
    // Presentation metadata follows the real event stream and survives audit
    // persistence without modifying the compaction or execution decisions.
    let start = done.audit[0].data["presentation"]["startedNanos"]
        .as_u64()
        .unwrap();
    let mut previous_elapsed = 0;
    let mut previous_offset = 0;
    for event in &done.audit {
        let presentation = &event.data["presentation"];
        assert_eq!(presentation["startedNanos"].as_u64(), Some(start));
        let elapsed = presentation["elapsedMillis"].as_u64().unwrap();
        let offset = presentation["textBytes"].as_u64().unwrap() as usize;
        assert!(elapsed >= previous_elapsed);
        assert!(offset >= previous_offset && offset <= done.assistant_text.len());
        assert!(done.assistant_text.is_char_boundary(offset));
        previous_elapsed = elapsed;
        previous_offset = offset;
    }
    let elapsed = done.events.last().unwrap().data["presentation"]["elapsedMillis"].clone();
    tokio::time::sleep(Duration::from_millis(15)).await;
    assert_eq!(
        agent.snapshot().unwrap().events.last().unwrap().data["presentation"]["elapsedMillis"],
        elapsed
    );

    assert_eq!(
        done.events
            .iter()
            .filter(|e| e.kind == "file_changes")
            .count(),
        1
    );
    assert_eq!(
        done.events
            .iter()
            .filter(|e| e.kind == "approval_required")
            .count(),
        1
    );
    assert_eq!(
        std::fs::read_to_string(root.0.join("created.rs")).unwrap(),
        "verified once"
    );
    assert!(done.transcript.len() > 40);
    assert!(done.audit.len() >= done.events.len());
    let seen = model.seen.lock().unwrap();
    for context in seen.iter().filter(|m| {
        m.iter().any(|v| {
            v["content"]
                .as_str()
                .unwrap_or("")
                .contains("Working checkpoint")
        })
    }) {
        let text = serde_json::to_string(context).unwrap();
        assert!(text.contains("preserve my goal"));
        assert!(text.contains("created.rs"));
        assert!(text.contains("verifiedFiles"));
        assert_tool_groups(context);
    }
}
fn assert_tool_groups(messages: &[Value]) {
    let mut pending = std::collections::HashSet::<String>::new();
    for message in messages {
        if message["role"] == "tool" {
            assert!(pending.remove(message["tool_call_id"].as_str().unwrap()));
        } else {
            assert!(pending.is_empty());
        }
        if let Some(calls) = message["tool_calls"].as_array() {
            for call in calls {
                pending.insert(call["id"].as_str().unwrap().into());
            }
        }
    }
    assert!(pending.is_empty());
}
struct StreamingMock;
impl Inference for StreamingMock {
    fn generate<'a>(&'a self, _: &'a [Value], _: &'a [Value]) -> FutureResult<'a, Turn> {
        unreachable!()
    }
    fn generate_stream<'a>(
        &'a self,
        _: &'a [Value],
        _: &'a [Value],
        updates: &'a (dyn Fn(ModelUpdate) + Send + Sync),
    ) -> FutureResult<'a, Turn> {
        Box::pin(async move {
            updates(ModelUpdate::Text("Actual partial".into()));
            tokio::time::sleep(Duration::from_secs(10)).await;
            unreachable!()
        })
    }
}
#[tokio::test]
async fn cancellation_keeps_actual_streamed_text_and_stops_future_work() {
    let root = Temp::new();
    let agent = Arc::new(Agent::default());
    let id = agent
        .start(
            "test",
            "hi",
            &root.0,
            Limits::default(),
            Arc::new(StreamingMock),
            Registry::default(),
        )
        .unwrap();
    let partial = wait(&agent, |v| !v.assistant_text.is_empty()).await;
    assert!(partial.generating);
    assert_eq!(partial.assistant_text, "Actual partial");
    agent.cancel("test", &id).unwrap();
    let done = wait(&agent, |v| !v.status.active()).await;
    assert_eq!(done.status, Status::Cancelled);
    assert_eq!(done.assistant_text, "Actual partial");
    assert!(!done.events.iter().any(|e| e.kind == "tool_started"));
}

#[tokio::test]
async fn discovery_references_survive_saved_followups_and_utf8_excerpts_can_expand() {
    let root = Temp::new();
    std::fs::create_dir(root.0.join("src")).unwrap();
    std::fs::write(
        root.0.join("src/large.rs"),
        format!("{}tail", "🐋".repeat(3000)),
    )
    .unwrap();
    let workspace = Workspace::new(&root.0).unwrap();
    workspace.restore_references(&[json!({"role":"tool","tool_name":"search_files","content":json!({"ok":true,"matches":[{"path":"src/large.rs"},{"path":"../escape"}]}).to_string()})]);
    assert!(workspace.require_reference("src/large.rs").is_ok());
    assert!(workspace.require_reference("../escape").is_err());
    let tool = Registry::default().get("read_file").unwrap();
    let (_, cancel) = tokio::sync::watch::channel(false);
    let first = tool
        .prepare(
            &workspace,
            json!({"path":"src/large.rs"}),
            &Limits::default(),
        )
        .unwrap();
    let first = tool
        .execute(
            &workspace,
            &first,
            &Limits::default(),
            cancel.clone(),
            &mut |_| {},
        )
        .await
        .unwrap();
    assert_eq!(first["nextOffset"], 8192);
    assert!(first["truncated"].as_bool().unwrap());
    let second = tool
        .prepare(
            &workspace,
            json!({"path":"src/large.rs","offset":8192}),
            &Limits::default(),
        )
        .unwrap();
    let second = tool
        .execute(&workspace, &second, &Limits::default(), cancel, &mut |_| {})
        .await
        .unwrap();
    assert!(second["content"].as_str().unwrap().ends_with("tail"));
    assert_eq!(second["truncated"], false);
}

struct RecoveringCommand(std::sync::atomic::AtomicUsize);
impl Tool for RecoveringCommand {
    fn schema(&self) -> Value {
        json!({"type":"function","function":{"name":"run_command"}})
    }
    fn prepare(
        &self,
        _: &Workspace,
        arguments: Value,
        _: &Limits,
    ) -> lorca::agent::Result<Prepared> {
        Ok(Prepared {
            action: arguments.clone(),
            approval: true,
            payload: arguments,
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
        Box::pin(async move {
            let successful = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0;
            Ok(
                json!({"ok":successful,"exitCode":if successful{0}else{1},"output":if successful{"test passed"}else{"test failed"}}),
            )
        })
    }
}
#[tokio::test]
async fn failed_command_can_run_again_after_a_verified_fix() {
    let root = Temp::new();
    std::fs::write(root.0.join("a.rs"), "broken").unwrap();
    let command = calls(vec![(
        "run_command",
        json!({"executable":"cargo","arguments":["test"]}),
    )]);
    let model = Mock::new(vec![
        command.clone(),
        calls(vec![(
            "apply_patch",
            json!({"path":"a.rs","old_text":"broken","new_text":"fixed"}),
        )]),
        command,
        finish(),
    ]);
    let counter = Arc::new(RecoveringCommand(std::sync::atomic::AtomicUsize::new(0)));
    let mut registry = Registry::default();
    registry.add("run_command", counter.clone());
    let agent = Arc::new(Agent::default());
    let id = agent
        .start(
            "test",
            "Run tests, fix, rerun tests",
            &root.0,
            Limits::default(),
            model,
            registry,
        )
        .unwrap();
    let mut previous = String::new();
    let mut approvals = 0;
    loop {
        let view = wait(&agent, |v| {
            !v.status.active() || v.approval.as_ref().is_some_and(|a| a.id != previous)
        })
        .await;
        if !view.status.active() {
            assert_eq!(view.status, Status::Completed, "{:?}", view.result);
            break;
        }
        let approval = view.approval.unwrap();
        previous = approval.id.clone();
        approvals += 1;
        agent
            .decide("test", &id, &approval.id, Decision::AllowOnce)
            .unwrap();
    }
    assert_eq!(approvals, 3);
    assert_eq!(counter.0.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(
        std::fs::read_to_string(root.0.join("a.rs")).unwrap(),
        "fixed"
    );
}
