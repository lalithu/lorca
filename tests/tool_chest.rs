use lorca::{
    agent::*,
    tools::registry::{Authority, Category},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("lorca-chest-{}", lorca::types::id()));
        std::fs::create_dir(&p).unwrap();
        std::fs::write(p.join("a.txt"), "before").unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn names(schemas: Vec<Value>) -> Vec<String> {
    schemas
        .iter()
        .map(|s| s["function"]["name"].as_str().unwrap().to_string())
        .collect()
}
#[test]
fn registry_metadata_and_drawers_are_real_and_legacy_names_work() {
    let r = Registry::default();
    let metadata = r.metadata();
    for (name, category, authority) in [
        ("read_file", Category::Files, Authority::WorkspaceRead),
        ("exec_command", Category::Terminal, Authority::Approval),
        ("fetch_url", Category::Web, Authority::PublicWeb),
        ("tool_search", Category::Agent, Authority::TaskState),
    ] {
        let entry = metadata.iter().find(|m| m.name == name).unwrap();
        assert_eq!(entry.category, category);
        assert_eq!(entry.authority, authority);
        assert!(!entry.description.is_empty());
        assert_eq!(
            entry.schema["function"]["parameters"]["additionalProperties"],
            false
        );
        assert!(r.get(name).is_ok());
    }
    assert!(r.get("run_command").is_ok());
    assert!(r.get("invented_tool").is_err());
    if std::env::var_os("LORCA_WEB_SEARCH_URL").is_none() {
        assert!(r.get("web_search").is_err());
        assert!(r.unavailable().contains_key("web_search"));
        assert!(!names(r.schemas()).contains(&"web_search".into()));
    }
}
#[tokio::test]
async fn discovery_expands_schemas_without_changing_authority() {
    let r = Registry::default();
    assert!(!names(r.active_schemas()).contains(&"git_diff".into()));
    let tool = r.get("tool_search").unwrap();
    let l = Limits::default();
    let (_tx, c) = watch::channel(false);
    let p = tool
        .prepare_optional(None, json!({"query":"git_diff"}), &l)
        .unwrap();
    let result = tool
        .execute_optional(None, &p, &l, c, &mut |_| {})
        .await
        .unwrap();
    assert_eq!(result["tools"][0]["name"], "git_diff");
    assert!(names(r.active_schemas()).contains(&"git_diff".into()));
    let tmp = Temp::new();
    let w = Workspace::new(&tmp.0).unwrap();
    let p = r
        .get("apply_patch")
        .unwrap()
        .prepare(
            &w,
            json!({"path":"a.txt","old_text":"before","new_text":"after"}),
            &l,
        )
        .unwrap();
    assert!(p.approval);
    assert_eq!(
        std::fs::read_to_string(tmp.0.join("a.txt")).unwrap(),
        "before"
    );
    assert!(
        r.get("read_file")
            .unwrap()
            .prepare_optional(None, json!({"path":"a.txt"}), &l)
            .is_err()
    );
}
#[test]
fn schemas_reject_unknown_arguments_and_invalid_plans() {
    let r = Registry::default();
    let l = Limits::default();
    for (name, args) in [
        ("tool_search", json!({"query":"files","approved":true})),
        (
            "request_permissions",
            json!({"reason":"test","scope":"all","granted":true}),
        ),
        ("request_user_input", json!({"question":""})),
        ("request_user_input", json!({"question":" None "})),
        ("request_user_input", json!({"question":"null"})),
        (
            "update_plan",
            json!({"steps":[{"step":"work","status":"made_up"}]}),
        ),
        (
            "update_plan",
            json!({"steps":[{"step":"a","status":"in_progress"},{"step":"b","status":"in_progress"}]}),
        ),
    ] {
        assert!(
            r.get(name)
                .unwrap()
                .prepare_optional(None, args, &l)
                .is_err(),
            "{name}"
        );
    }
    let t = Temp::new();
    let w = Workspace::new(&t.0).unwrap();
    for name in ["run_command", "exec_command"] {
        let prepared = r.get(name).unwrap().prepare(
            &w,
            json!({"executable":"sh","arguments":["-c","echo hello"]}),
            &l,
        );
        if name == "exec_command" {
            assert!(prepared.unwrap().approval);
        } else {
            assert!(prepared.is_err());
        }
        assert!(
            r.get(name)
                .unwrap()
                .prepare(
                    &w,
                    json!({"executable":"cargo","arguments":["test"],"directory":"../"}),
                    &l
                )
                .is_err()
        );
    }
    assert!(
        r.get("write_stdin")
            .unwrap()
            .prepare(&w, json!({"session_id":"another-task"}), &l)
            .is_err()
    );
}
#[test]
fn web_preparation_rejects_private_urls_before_approval() {
    let r = Registry::default();
    let tool = r.get("fetch_url").unwrap();
    for url in [
        "file:///etc/passwd",
        "http://example.com",
        "https://localhost/",
        "https://127.0.0.1/",
        "https://[::1]/",
        "https://192.168.0.1/",
        "https://100.64.0.1/",
        "https://user:secret@example.com/",
        "https://example.com:8443/",
    ] {
        assert!(
            tool.prepare_optional(None, json!({"url":url}), &Limits::default())
                .is_err(),
            "{url}"
        );
    }
    assert!(
        tool.prepare_optional(
            None,
            json!({"url":"https://example.com/"}),
            &Limits::default()
        )
        .unwrap()
        .approval
    );
}
struct Mock(Mutex<VecDeque<Turn>>);
impl Mock {
    fn new(calls: Vec<(&str, Value)>) -> Arc<Self> {
        Arc::new(Self(Mutex::new(VecDeque::from([lorca::provider::agent::parse_turn(json!({"role":"assistant","content":"","tool_calls":calls.into_iter().map(|(n,a)|json!({"function":{"name":n,"arguments":a}})).collect::<Vec<_>>()})).unwrap()]))))
    }
}
impl Inference for Mock {
    fn generate<'a>(&'a self, _: &'a [Value], _: &'a [Value]) -> FutureResult<'a, Turn> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .pop_front()
                .ok_or("Unexpected inference after pause".into())
        })
    }
}
async fn settled(agent: &Agent) -> TaskView {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(v) = agent.snapshot()
                && !v.status.active()
            {
                return v;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn user_question_pauses_without_workspace_or_assumed_answer() {
    let agent = Arc::new(Agent::default());
    agent
        .start(
            "test",
            "Help me decide",
            std::path::Path::new(""),
            Limits::default(),
            Mock::new(vec![(
                "request_user_input",
                json!({"question":"Which file should I inspect?"}),
            )]),
            Registry::default(),
        )
        .unwrap();
    let result = settled(&agent).await;
    assert_eq!(result.status, Status::WaitingForInput);
    assert!(result.result.unwrap().contains("Which file"));
    assert!(result.events.iter().any(|e| e.kind == "tool_completed"));
}
#[tokio::test]
async fn permission_request_never_grants_scope_or_executes_later_calls() {
    let tmp = Temp::new();
    let agent = Arc::new(Agent::default());
    agent
        .start(
            "test",
            "Update a.txt",
            &tmp.0,
            Limits::default(),
            Mock::new(vec![
                (
                    "request_permissions",
                    json!({"reason":"Need another directory","scope":"outside workspace"}),
                ),
                (
                    "apply_patch",
                    json!({"path":"a.txt","old_text":"before","new_text":"bad"}),
                ),
            ]),
            Registry::default(),
        )
        .unwrap();
    let result = settled(&agent).await;
    assert_eq!(result.status, Status::WaitingForInput);
    assert_eq!(
        std::fs::read_to_string(tmp.0.join("a.txt")).unwrap(),
        "before"
    );
    let permission = result
        .events
        .iter()
        .find(|e| e.kind == "tool_completed")
        .unwrap();
    assert_eq!(permission.data["result"]["granted"], false);
}
#[tokio::test]
async fn discovery_does_not_bypass_task_restrictions_for_new_command_name() {
    let tmp = Temp::new();
    let agent = Arc::new(Agent::default());
    agent
        .start(
            "test",
            "Read only; do not run commands",
            &tmp.0,
            Limits::default(),
            Mock::new(vec![(
                "exec_command",
                json!({"executable":"cargo","arguments":["test"]}),
            )]),
            Registry::default(),
        )
        .unwrap();
    let result = settled(&agent).await;
    assert!(
        !result
            .events
            .iter()
            .any(|e| e.kind == "approval_required" || e.kind == "tool_started")
    );
    assert!(result.events.iter().any(|e| {
        e.kind == "tool_failed"
            && e.data["result"]["error"]
                .as_str()
                .unwrap_or("")
                .contains("restriction")
    }));
}

#[test]
fn completed_session_exit_is_valid_verification_but_running_acknowledgement_is_not() {
    use lorca::agentloop::verification::Requirements;
    let r = Requirements::new("Run cargo test");
    let start = json!({"role":"assistant","tool_calls":[{"id":"exec","function":{"name":"exec_command","arguments":{"executable":"cargo","arguments":["test"]}}}]});
    let pending = json!({"role":"tool","tool_name":"exec_command","tool_call_id":"exec","content":json!({"ok":true,"sessionId":"s","running":true}).to_string()});
    assert!(!r.missing(&[start.clone(), pending.clone()]).is_empty());
    let finished = json!({"role":"tool","tool_name":"write_stdin","tool_call_id":"poll","content":json!({"ok":true,"sessionId":"s","running":false,"exitCode":0,"arguments":["test"]}).to_string()});
    assert!(r.missing(&[start, pending, finished]).is_empty());
}

#[test]
fn session_started_before_an_edit_does_not_count_as_fresh_verification() {
    use lorca::agentloop::verification::Requirements;
    let r = Requirements::new("Update a.txt and run cargo test");
    let start = json!({"role":"tool","tool_name":"exec_command","tool_call_id":"start","content":json!({"ok":true,"sessionId":"old","running":true}).to_string()});
    let patch = json!({"role":"tool","tool_name":"apply_patch","tool_call_id":"patch","content":json!({"ok":true,"verified":true,"path":"a.txt"}).to_string()});
    let finish = json!({"role":"tool","tool_name":"write_stdin","tool_call_id":"poll","content":json!({"ok":true,"sessionId":"old","running":false,"exitCode":0,"arguments":["test"]}).to_string()});
    assert!(
        !r.missing(&[start.clone(), patch.clone(), finish.clone()])
            .is_empty()
    );
    assert!(r.missing(&[patch, start, finish]).is_empty());
}
#[tokio::test]
async fn explicit_network_restriction_cannot_be_expanded_by_approval() {
    let tmp = Temp::new();
    let agent = Arc::new(Agent::default());
    agent
        .start(
            "test",
            "Inspect offline only; no network",
            &tmp.0,
            Limits::default(),
            Mock::new(vec![("fetch_url", json!({"url":"https://example.com"}))]),
            Registry::default(),
        )
        .unwrap();
    let v = settled(&agent).await;
    assert!(
        !v.events
            .iter()
            .any(|e| e.kind == "approval_required" || e.kind == "tool_started")
    );
    assert!(v.events.iter().any(|e| e.kind == "tool_failed"));
}

#[test]
fn registered_terminal_schema_does_not_coerce_malformed_arguments() {
    let root = Temp::new();
    let w = Workspace::new(&root.0).unwrap();
    let r = Registry::default();
    assert!(names(r.active_schemas()).contains(&"exec_command".into()));
    let schemas = r.active_schemas();
    let schema = schemas
        .iter()
        .find(|s| s["function"]["name"] == "exec_command")
        .unwrap();
    assert_eq!(
        schema["function"]["parameters"]["properties"]["arguments"]["type"],
        "array"
    );
    assert_eq!(
        schema["function"]["parameters"]["additionalProperties"],
        false
    );
    let tool = r.get("exec_command").unwrap();
    for args in [
        json!({"command":"cd /tmp && cargo test"}),
        json!({"arguments":"[\"cargo\",\"test\"]","directory":"."}),
        json!({"executable":"cargo","arguments":"[\"test\"]"}),
    ] {
        assert!(tool.prepare(&w, args, &Limits::default()).is_err());
    }
}

#[tokio::test]
async fn multi_file_patch_reviews_and_verifies_multiple_hunks_creation_and_deletion() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "one\nkeep\nthree\n").unwrap();
    std::fs::write(t.0.join("obsolete.txt"), "obsolete\n").unwrap();
    let w = Workspace::new(&t.0).unwrap();
    let tool = Registry::default().get("apply_patch").unwrap();
    let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-one\n+first\n@@\n-three\n+last\n*** Add File: new.txt\n+created\n*** Delete File: obsolete.txt\n*** End Patch";
    let l = Limits::default();
    let proposal = tool.prepare(&w, json!({"patch":patch}), &l).unwrap();
    assert!(proposal.approval);
    let preview = proposal.action["diff"].as_str().unwrap();
    assert!(preview.contains("@@ -1,1 +1,1 @@\n-one\n+first"));
    assert!(preview.contains("@@ -3,1 +3,1 @@\n-three\n+last"));
    assert!(preview.contains("+++ /dev/null"));
    assert!(!t.0.join("new.txt").exists());
    let (_tx, c) = watch::channel(false);
    let result = tool
        .execute(&w, &proposal, &l, c, &mut |_| {})
        .await
        .unwrap();
    assert_eq!(result["verified"], true);
    assert_eq!(result["changes"].as_array().unwrap().len(), 3);
    assert_eq!(
        std::fs::read_to_string(t.0.join("a.txt")).unwrap(),
        "first\nkeep\nlast\n"
    );
    assert_eq!(
        std::fs::read_to_string(t.0.join("new.txt")).unwrap(),
        "created\n"
    );
    assert!(!t.0.join("obsolete.txt").exists());
}

#[tokio::test]
async fn patch_rejects_ambiguous_context_and_rechecks_every_file_after_approval() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "same\nsame\n").unwrap();
    std::fs::write(t.0.join("b.txt"), "before\n").unwrap();
    let w = Workspace::new(&t.0).unwrap();
    let tool = Registry::default().get("apply_patch").unwrap();
    let l = Limits::default();
    let ambiguous = "*** Begin Patch\n*** Update File: a.txt\n@@\n-same\n+changed\n*** End Patch";
    assert!(
        tool.prepare(&w, json!({"patch":ambiguous}), &l)
            .err()
            .unwrap()
            .contains("ambiguous")
    );
    let patch = "*** Begin Patch\n*** Add File: new.txt\n+created\n*** Update File: b.txt\n@@\n-before\n+after\n*** End Patch";
    let proposal = tool.prepare(&w, json!({"patch":patch}), &l).unwrap();
    std::fs::write(t.0.join("b.txt"), "external edit\n").unwrap();
    let (_tx, c) = watch::channel(false);
    let error = tool
        .execute(&w, &proposal, &l, c, &mut |_| {})
        .await
        .unwrap_err();
    assert!(error.contains("changed since proposal"));
    assert!(!t.0.join("new.txt").exists());
    assert_eq!(
        std::fs::read_to_string(t.0.join("b.txt")).unwrap(),
        "external edit\n"
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn patch_rolls_back_prior_files_after_a_late_write_failure() {
    use std::os::unix::fs::PermissionsExt;
    let t = Temp::new();
    let locked = t.0.join("locked");
    std::fs::create_dir(&locked).unwrap();
    let w = Workspace::new(&t.0).unwrap();
    let tool = Registry::default().get("apply_patch").unwrap();
    let l = Limits::default();
    let patch = "*** Begin Patch\n*** Delete File: a.txt\n*** Add File: locked/new.txt\n+created\n*** End Patch";
    let proposal = tool.prepare(&w, json!({"patch":patch}), &l).unwrap();
    let mode = std::fs::metadata(t.0.join("a.txt"))
        .unwrap()
        .permissions()
        .mode();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
    let (_tx, c) = watch::channel(false);
    let result = tool.execute(&w, &proposal, &l, c, &mut |_| {}).await;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(result.unwrap_err().contains("locked/new.txt"));
    assert_eq!(
        std::fs::read_to_string(t.0.join("a.txt")).unwrap(),
        "before"
    );
    assert_eq!(
        std::fs::metadata(t.0.join("a.txt"))
            .unwrap()
            .permissions()
            .mode(),
        mode
    );
    assert!(!locked.join("new.txt").exists());
}

#[tokio::test]
async fn explicit_paths_remain_inside_the_granted_workspace() {
    let t = Temp::new();
    let other = Temp::new();
    let w = Workspace::new(&t.0).unwrap();
    let tool = Registry::default().get("read_file").unwrap();
    let l = Limits::default();
    let proposal = tool
        .prepare(&w, json!({"path":w.root.join("a.txt")}), &l)
        .unwrap();
    let (_tx, c) = watch::channel(false);
    let result = tool
        .execute(&w, &proposal, &l, c, &mut |_| {})
        .await
        .unwrap();
    assert_eq!(result["content"], "before");
    let error = tool
        .prepare(
            &w,
            json!({"path":other.0.canonicalize().unwrap().join("a.txt")}),
            &l,
        )
        .err()
        .unwrap();
    assert!(error.contains("request_workspace"));
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn general_command_runs_only_after_exact_approval_and_denial_cannot_run_a_batch() {
    for decision in [Decision::AllowOnce, Decision::Deny] {
        let t = Temp::new();
        let agent = Arc::new(Agent::default());
        let mock = Mock::new(vec![(
            "exec_command",
            json!({"executable":"sh","arguments":["-c","printf ok > command.txt"]}),
        )]);
        let final_turn = if matches!(decision, Decision::Deny) {
            // Even if the model requests a different write during reconsideration,
            // the denial follow-up must never dispatch a tool.
            lorca::provider::agent::parse_turn(json!({"role":"assistant","tool_calls":[{"function":{"name":"apply_patch","arguments":{"path":"alternate.txt","old_text":"","new_text":"bad"}}}]})).unwrap()
        } else {
            lorca::provider::agent::parse_turn(
                json!({"role":"assistant","content":"Command finished."}),
            )
            .unwrap()
        };
        mock.0.lock().unwrap().push_back(final_turn);
        let id = agent
            .start(
                "test",
                "Run the fixture command",
                &t.0,
                Limits::default(),
                mock,
                Registry::default(),
            )
            .unwrap();
        let pending = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(approval) = agent.snapshot().and_then(|v| v.approval) {
                    break approval;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(!t.0.join("command.txt").exists());
        assert_eq!(
            pending.action["arguments"],
            json!(["-c", "printf ok > command.txt"])
        );
        let denied = matches!(decision, Decision::Deny);
        agent.decide("test", &id, &pending.id, decision).unwrap();
        let outcome = settled(&agent).await;
        if denied {
            assert_eq!(outcome.status, Status::Denied);
            assert!(!t.0.join("command.txt").exists());
            assert!(!t.0.join("alternate.txt").exists());
            assert!(
                !outcome
                    .events
                    .iter()
                    .any(|event| event.kind == "tool_started")
            );
        } else {
            assert_eq!(outcome.status, Status::Completed);
            assert_eq!(
                std::fs::read_to_string(t.0.join("command.txt")).unwrap(),
                "ok"
            );
            assert!(
                outcome
                    .events
                    .iter()
                    .any(|event| event.data["result"]["exitCode"] == 0)
            );
        }
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn git_diff_and_status_work_in_a_linked_worktree_with_scoped_metadata() {
    use std::process::Command;
    let base = Temp::new();
    let repo = base.0.join("repo");
    let linked = base.0.join("linked");
    std::fs::create_dir(&repo).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("/usr/bin/git")
            .current_dir(&repo)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    std::fs::write(repo.join("file.txt"), "base\n").unwrap();
    git(&["add", "file.txt"]);
    git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "commit",
        "-qm",
        "base",
    ]);
    git(&[
        "worktree",
        "add",
        "-qb",
        "fixture-linked",
        linked.to_str().unwrap(),
    ]);
    std::fs::write(linked.join("file.txt"), "staged\n").unwrap();
    git(&["-C", linked.to_str().unwrap(), "add", "file.txt"]);
    std::fs::write(linked.join("file.txt"), "working\n").unwrap();

    let workspace = Workspace::new(&linked).unwrap();
    let registry = Registry::default();
    let tool = registry.get("git_diff").unwrap();
    let limits = Limits::default();
    let (_tx, c) = watch::channel(false);
    for (staged, expected) in [(false, "+working"), (true, "+staged")] {
        let prepared = tool
            .prepare(&workspace, json!({"staged":staged}), &limits)
            .unwrap();
        assert!(prepared.approval);
        assert!(
            prepared.action["scope"]
                .as_str()
                .unwrap()
                .contains("/repo/.git")
        );
        let result = tool
            .execute(&workspace, &prepared, &limits, c.clone(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(result["exitCode"], 0, "{result}");
        assert!(result["output"].as_str().unwrap().contains(expected));
    }
    let status = registry.get("exec_command").unwrap();
    let prepared = status.prepare(&workspace,json!({"executable":"git","arguments":["status","--short"],"read_only":true,"git_metadata":true}),&limits).unwrap();
    assert!(prepared.approval);
    let result = status
        .execute(&workspace, &prepared, &limits, c, &mut |_| {})
        .await
        .unwrap();
    assert_eq!(result["exitCode"], 0, "{result}");
    assert!(result["output"].as_str().unwrap().contains("MM file.txt"));
}
