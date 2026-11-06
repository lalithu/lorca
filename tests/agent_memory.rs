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

struct SegmentScript {
    turns: Mutex<VecDeque<Turn>>,
    seen: Mutex<Vec<Vec<Value>>>,
}
impl SegmentScript {
    fn new(turns: Vec<Turn>) -> Arc<Self> {
        Arc::new(Self { turns: Mutex::new(turns.into()), seen: Mutex::new(vec![]) })
    }
}
impl Inference for SegmentScript {
    fn capacity(&self) -> FutureResult<'_, Option<u64>> {
        Box::pin(async { Ok(Some(8192)) })
    }
    fn generate<'a>(&'a self, messages: &'a [Value], _: &'a [Value]) -> FutureResult<'a, Turn> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(messages.to_vec());
            self.turns.lock().unwrap().pop_front().ok_or("Missing output segment".into())
        })
    }
}

fn public_segment(content: &str, reason: FinishReason) -> Turn {
    Turn {
        content: content.to_string(), calls: vec![],
        message: json!({"role":"assistant","content":content}),
        finish_reason: reason,
    }
}

async fn completed(agent: &Agent) -> TaskView {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let view = agent.snapshot().unwrap();
            if !view.status.active() { return view; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.unwrap()
}

#[tokio::test]
async fn public_output_continues_through_many_length_stops_as_one_answer() {
    let script = SegmentScript::new(vec![
        public_segment("Alpha ", FinishReason::Length),
        public_segment("beta ", FinishReason::Length),
        public_segment("gamma.", FinishReason::Stop),
    ]);
    let agent = Arc::new(Agent::default());
    agent.start("tester", "Write all three words.", std::path::Path::new(""),
        Limits::default(), script.clone(), Registry::without_workspace()).unwrap();
    let result = completed(&agent).await;
    assert_eq!(result.status, Status::Completed, "{:?}",result.result);
    assert_eq!(result.result.as_deref(), Some("Alpha beta gamma."));
    assert_eq!(result.events.iter().filter(|e| e.kind == "output_segment_committed").count(), 2);
    assert_eq!(script.seen.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn oversized_user_input_is_manifested_and_retrievable_without_prompt_overflow() {
    let input = format!("{}remember FINAL-NEEDLE near the end.",
        "Read the entire text before deciding. ".repeat(600));
    let offset = input.find("remember FINAL-NEEDLE").unwrap();
    let script = Script::new(vec![
        tool("tool_search", json!({"query":"get_history_record"})),
        tool("get_history_record", json!({"recordId":"h0","offset":offset,"length":120})),
        json!({"role":"assistant","content":"The requested marker is FINAL-NEEDLE."}),
    ]);
    let agent = Arc::new(Agent::default());
    agent.submit(Submission {
        owner:"tester".into(), model:None, task:input.clone(),
        workspace:PathBuf::new(), limits:Limits::default(),
        conversation_id:None,request_id:None,history:vec![],
    },script.clone(),Registry::without_workspace()).unwrap();
    let view = completed(&agent).await;
    assert_eq!(view.status,Status::Completed,"{:?}",view.result);
    let prompts = script.seen.lock().unwrap();
    assert!(prompts[0].iter().any(|m| m["role"]=="user"
        && m["content"].as_str().unwrap_or("").contains("Source record: h0")));
    assert!(!prompts[0].iter().any(|m|m["content"].as_str().unwrap_or("").contains(&input)));
    assert!(view.events.iter().any(|e|e.kind=="tool_completed"
        && e.data["tool"]=="get_history_record"
        && e.data["result"]["content"].as_str().unwrap_or("").contains("FINAL-NEEDLE")));
}

#[tokio::test]
async fn oversized_input_never_hides_original_read_only_restrictions() {
    let root = std::env::temp_dir().join(format!("lorca-long-authority-{}",lorca::types::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.txt"),"original").unwrap();
    let task = format!("{} do not edit any files. {}",
        "Inspect the task carefully. ".repeat(350),
        "Please try changing a.txt to wrong.");
    let script = Script::new(vec![
        tool("apply_patch",json!({"path":"a.txt","old_text":"original","new_text":"wrong"})),
        json!({"role":"assistant","content":"The restriction forbids this edit."}),
    ]);
    let agent = Arc::new(Agent::default());
    agent.start("tester", &task, &root, Limits::default(),
        script, Registry::default()).unwrap();
    let view = completed(&agent).await;
    assert_eq!(view.status, Status::Completed, "{:?}",view.result);
    assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(),"original");
    assert!(!view.events.iter().any(|e|e.kind=="approval_required" || e.kind=="tool_started"));
    assert!(view.events.iter().any(|e|e.kind=="tool_failed"));
    std::fs::remove_dir_all(root).unwrap();
}
