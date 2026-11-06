//! Explicit opt-in, real-provider archive recall. No file writes or approvals.
use lorca::{
    agent::*,
    provider::{Engine, llama_cpp::LocalAgent},
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc, time::Duration};
#[tokio::test]
#[ignore = "Requires LORCA_LIVE_MODEL and an available local provider; real inference"]
async fn real_model_recalls_original_after_checkpoint() {
    let model = std::env::var("LORCA_LIVE_MODEL").expect("Set LORCA_LIVE_MODEL");
    let engine = match std::env::var("LORCA_LIVE_PROVIDER").as_deref() {
        Ok("llama_cpp") => Engine::LlamaCpp,
        _ => Engine::Ollama,
    };
    let host = std::env::var("LORCA_LIVE_HOST").unwrap_or_else(|_| engine.default_host().into());
    let mut history = vec![json!({"role":"user","content":"Our original marker is alpine-742."})];
    for n in 0..100 {
        history.push(json!({"role":"assistant","content":format!("Past exchange {n}: {}","ordinary unrelated conversation ".repeat(50))}));
    }
    let model = Arc::new(LocalAgent {
        client: reqwest::Client::new(),
        host,
        model,
        engine,
    });
    let agent = Arc::new(Agent::default());
    let start = std::time::Instant::now();
    agent.submit(Submission{owner:"v05-live".into(),task:"Retrieve the exact marker from original user record h0 with get_history_record. Discover the tool if needed. Reply with the marker only. Do not guess.".into(),
        model:None,workspace:PathBuf::new(),limits:Limits{iterations:10,..Default::default()},conversation_id:None,request_id:None,history},model,Registry::without_workspace()).unwrap();
    let view = tokio::time::timeout(Duration::from_secs(240), async {
        loop {
            let v = agent.snapshot().unwrap();
            if !v.status.active() {
                break v;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    println!(
        "LIVE_ARCHIVE status={:?} seconds={:.3} usage={:?} result={:?}",
        view.status,
        start.elapsed().as_secs_f64(),
        view.usage,
        view.result
    );
    assert_eq!(view.status, Status::Completed);
    assert!(view.result.as_deref().unwrap_or("").contains("alpine-742"));
    assert!(view.events.iter().any(|e| e.kind == "context_compacted"));
    assert!(view.events.iter().any(|e| e.kind == "tool_completed"
        && e.data["tool"] == "get_history_record"
        && e.data["result"]["recordId"] == "h0"));
    assert!(!view.events.iter().any(|e| e.kind == "approval_required"));
}
