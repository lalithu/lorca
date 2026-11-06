//! End-to-end recall regression: original historical records remain reachable
//! after model-facing context compaction, without a second model or approval.
use lorca::{agent::*, provider::agent::parse_turn};
use serde_json::{Value, json};
use std::{collections::VecDeque, path::PathBuf, sync::{Arc, Mutex}, time::Duration};

struct Script {
    turns: Mutex<VecDeque<Value>>,
    seen: Mutex<Vec<Vec<Value>>>,
}
impl Script {
    fn new(turns: Vec<Value>) -> Arc<Self> {
        Arc::new(Self { turns: Mutex::new(turns.into()), seen: Mutex::new(Vec::new()) })
    }
}
impl Inference for Script {
    fn capacity(&self) -> FutureResult<'_, Option<u64>> {
        Box::pin(async { Ok(Some(8192)) })
    }
    fn generate<'a>(&'a self, messages: &'a [Value], _: &'a [Value]) -> FutureResult<'a, Turn> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(messages.to_vec());
            parse_turn(self.turns.lock().unwrap().pop_front().ok_or("Missing scripted turn")?)
        })
    }
}
fn tool(name: &str, args: Value) -> Value {
    json!({"role":"assistant","tool_calls":[{
        "function":{"name":name,"arguments":args}}]})
}
#[tokio::test]
async fn old_original_records_are_retrievable_after_context_compaction() {
    let mut history = vec![json!({"role":"user","content":"Remember alpine-742 as the original answer."})];
    for _ in 0..22 {
        history.push(json!({"role":"assistant","content":"Ordinary archived conversation. ".repeat(250)}));
    }
    history.push(json!({"role":"user","content":"Find the original marker in older history."}));
    let script = Script::new(vec![
        tool("tool_search", json!({"query":"history"})),
        tool("search_history", json!({"query":"alpine-742"})),
        tool("get_history_record", json!({"recordId":"h0"})),
        json!({"role":"assistant","content":"I recovered the original marker: alpine-742."}),
    ]);
    let agent = Arc::new(Agent::default());
    let task = agent.submit(
        Submission {
            owner: "tester".into(), model: None,
            task: "Find the original marker in older history.".into(),
            workspace: PathBuf::new(),
            limits: Limits::default(), conversation_id: None, request_id: None, history,
        },
        script.clone(),
        Registry::without_workspace(),
    ).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = agent.snapshot().unwrap();
            if !snapshot.status.active() { return snapshot; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.unwrap();
    assert_eq!(result.id, task);
    assert_eq!(result.status, Status::Completed, "{:?}", result.result);
    assert!(result.events.iter().any(|e| e.kind == "context_compacted"));
    let calls: Vec<_> = result.events.iter().filter(|e| e.kind == "tool_completed").collect();
    assert!(calls.iter().any(|e| e.data["tool"] == "search_history" &&
        e.data["result"]["matches"][0]["recordId"] == "h0"));
    assert!(calls.iter().any(|e| e.data["tool"] == "get_history_record" &&
        e.data["result"]["content"].as_str().unwrap_or("").contains("alpine-742")));
    assert!(script.seen.lock().unwrap().len() >= 4);
    assert!(!result.events.iter().any(|e| e.kind == "approval_required"));
}
