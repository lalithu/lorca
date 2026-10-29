//! Opt-in, read-only local smoke test. It requests tool calls but executes none.
use lorca::{
    agent::{FinishReason, Registry, ThinkingEffort},
    context::GenerationBudget,
    provider::agent::OllamaAgent,
};
use serde_json::json;
#[tokio::test]
#[ignore = "requires an explicitly selected, installed local model"]
async fn installed_model_returns_structured_calls_with_each_effort() {
    let model =
        std::env::var("LORCA_LIVE_MODEL").expect("Set LORCA_LIVE_MODEL to an installed model");
    let provider = OllamaAgent {
        client: reqwest::Client::new(),
        host: "http://127.0.0.1:11434".into(),
        model: model.clone(),
    };
    let schema = Registry::default().get("read_file").unwrap().schema();
    for effort in [
        ThinkingEffort::Off,
        ThinkingEffort::Nominal,
        ThinkingEffort::High,
    ] {
        let turn = provider.generate_for_effort(&[
            json!({"role":"system","content":"Use a native tool call with arguments matching the provided schema. Do not invent results."}),
            json!({"role":"user","content":"Call read_file to read README.md. Do not answer with prose; make only this tool call."})
        ],std::slice::from_ref(&schema),GenerationBudget {context_limit:32768,output_limit:1024,safety_margin:512},effort,&|_|{}).await.unwrap();
        assert_ne!(
            turn.finish_reason,
            FinishReason::Length,
            "{model}: {effort:?} exhausted the smoke-test allowance"
        );
        assert_eq!(
            turn.calls.len(),
            1,
            "{model}: {effort:?}: expected one complete tool call"
        );
        assert_eq!(turn.calls[0].name, "read_file");
        assert_eq!(turn.calls[0].arguments["path"], "README.md");
        println!("PASS {model} {effort:?}: one complete read_file call; no tool executed");
    }
}
