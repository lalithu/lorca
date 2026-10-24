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
            "Inspect and verify",
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
    assert_eq!(view.status, Status::Failed);
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
