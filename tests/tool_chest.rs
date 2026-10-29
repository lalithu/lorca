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
    assert_eq!(r.active_schemas().len(), 8);
    let tool = r.get("tool_search").unwrap();
    let l = Limits::default();
    let (_tx, c) = watch::channel(false);
    let p = tool
        .prepare_optional(None, json!({"query":"apply_patch"}), &l)
        .unwrap();
    let result = tool
        .execute_optional(None, &p, &l, c, &mut |_| {})
        .await
        .unwrap();
    assert_eq!(result["tools"][0]["name"], "apply_patch");
    assert!(names(r.active_schemas()).contains(&"apply_patch".into()));
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
        assert!(
            r.get(name)
                .unwrap()
                .prepare(
                    &w,
                    json!({"executable":"sh","arguments":["-c","echo bad"]}),
                    &l
                )
                .is_err()
        );
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
fn terminal_schema_is_available_upfront_and_does_not_coerce_malformed_arguments() {
    let root = Temp::new();
    let w = Workspace::new(&root.0).unwrap();
    let r = Registry::default();
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
        json!({"executable":"sh","arguments":["-c","cargo test"]}),
    ] {
        assert!(tool.prepare(&w, args, &Limits::default()).is_err());
    }
}
