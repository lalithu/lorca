use lorca::agent::*;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
struct Model;
impl Inference for Model {
    fn capacity(&self) -> FutureResult<'_, Option<u64>> {
        Box::pin(async { Ok(Some(8192)) })
    }
    fn generate<'a>(&'a self, m: &'a [Value], t: &'a [Value]) -> FutureResult<'a, Turn> {
        Box::pin(async move {
            let interpreting = t.iter().any(|t| t["function"]["name"] == "interpret_task");
            let text = m.iter().map(Value::to_string).collect::<String>();
            let revised = text.contains("Actually");
            let message = if interpreting {
                json!({"role":"assistant","tool_calls":[{"id":"intent","function":{"name":"interpret_task","arguments":{"relation":if revised {"correct"} else {"new"},"objective":if revised {"Create chosen.txt containing NEW"} else {"Create chosen.txt containing OLD"},"outcomes":[{"kind":"edit","target":"chosen.txt"}],"edit_targets":["chosen.txt"],"forbidden_targets":[],"read_only":false,"commands_forbidden":true,"clarification":"None"}}}]})
            } else if m.iter().any(|v| {
                v["role"] == "tool"
                    && v["content"]
                        .as_str()
                        .unwrap_or("")
                        .contains("\"verified\":true")
            }) {
                json!({"role":"assistant","content":"Created chosen.txt."})
            } else {
                json!({"role":"assistant","tool_calls":[{"id":if revised {"new"} else {"old"},"function":{"name":"apply_patch","arguments":{"path":"chosen.txt","old_text":"","new_text":if revised {"NEW"} else {"OLD"}}}}]})
            };
            lorca::provider::agent::parse_turn(message)
        })
    }
}
async fn approval(a: &Agent) -> Approval {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(v) = a.snapshot()
                && let Some(p) = v.approval
            {
                return p;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
async fn revision_case(grant_first: bool) {
    let dir = std::env::temp_dir().join(format!("lorca-revision-{}", lorca::types::id()));
    std::fs::create_dir(&dir).unwrap();
    let a = Arc::new(Agent::default());
    let id = a
        .start(
            "owner",
            "Create chosen.txt containing OLD. Do not run commands.",
            &dir,
            Limits::default(),
            Arc::new(Model),
            Registry::default(),
        )
        .unwrap();
    let old = approval(&a).await;
    assert!(
        a.revise("other", &id, "Actually create chosen.txt containing NEW.")
            .is_err()
    );
    if grant_first {
        a.decide("owner", &id, &old.id, Decision::AllowOnce)
            .unwrap();
    }
    a.revise("owner", &id, "Actually create chosen.txt containing NEW.")
        .unwrap();
    assert!(
        a.decide("owner", &id, &old.id, Decision::AllowOnce)
            .is_err()
    );
    let new = approval(&a).await;
    assert_ne!(old.id, new.id);
    assert_eq!(new.action["objectiveRevision"], 2);
    assert_eq!(a.snapshot().unwrap().id, id);
    assert!(!dir.join("chosen.txt").exists());
    a.decide("owner", &id, &new.id, Decision::AllowOnce)
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if !a.snapshot().unwrap().status.active() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let v = a.snapshot().unwrap();
    assert_eq!(v.status, Status::Completed, "{:?}", v.result);
    assert_eq!(
        std::fs::read_to_string(dir.join("chosen.txt")).unwrap(),
        "NEW"
    );
    assert_eq!(
        v.audit
            .iter()
            .filter(|e| e.kind == "tool_started" && e.data["tool"] == "apply_patch")
            .count(),
        1
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn core_status_does_not_replace_task_or_consume_approval() {
    let root = std::env::temp_dir().join(format!("lorca-status-{}", lorca::types::id()));
    std::fs::create_dir(&root).unwrap();
    let core = lorca::core::Core::new(lorca::storage::Repository::new(root.join("data")).unwrap())
        .unwrap();
    let session = core
        .command(json!({"op":"connect","model":"fixture"}))
        .await
        .unwrap()["session"]
        .as_str()
        .unwrap()
        .to_owned();
    let id = core
        .agent()
        .start(
            &session,
            "Create chosen.txt containing OLD.",
            &root,
            Limits::default(),
            Arc::new(Model),
            Registry::default(),
        )
        .unwrap();
    let p = approval(core.agent()).await;
    let status = core
        .command(json!({"op":"agent_status","session":session}))
        .await
        .unwrap();
    assert_eq!(status["taskId"], id);
    assert_eq!(status["revision"], 1);
    assert_eq!(core.agent().snapshot().unwrap().approval.unwrap().id, p.id);
    core.agent().cancel(&session, &id).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn correction_preserves_identity_invalidates_approval_and_executes_only_new_action() {
    revision_case(false).await;
}
#[tokio::test]
async fn granted_but_not_started_approval_cannot_execute_after_revision() {
    revision_case(true).await;
}
