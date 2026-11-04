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

#[tokio::test]
#[ignore = "requires the installed local model; prints counts, never reasoning text"]
async fn thinking_visibility_on_off_identical_prompt() {
    use lorca::agent::ModelUpdate;
    use std::{sync::Mutex, time::Instant};
    let model = std::env::var("LORCA_LIVE_MODEL").expect("Set the installed model identifier");
    let provider = OllamaAgent {
        client: reqwest::Client::new(),
        host: "http://127.0.0.1:11434".into(),
        model,
    };
    let messages = [
        json!({"role":"user","content":"What is 17 multiplied by 23? Answer with the integer only."}),
    ];
    for effort in [ThinkingEffort::Off, ThinkingEffort::Nominal] {
        let stats = Mutex::new((0usize, None::<lorca::types::Usage>));
        let updates = |update| {
            let mut stats = stats.lock().unwrap();
            match update {
                ModelUpdate::Reasoning { state, text, .. } if state == "delta" => {
                    stats.0 += text.len()
                }
                ModelUpdate::Usage(usage) => stats.1 = Some(usage),
                _ => {}
            }
        };
        let start = Instant::now();
        let turn = provider
            .generate_for_effort(
                &messages,
                &[],
                GenerationBudget::new(65536, false),
                effort,
                &updates,
            )
            .await
            .unwrap();
        let stats = stats.lock().unwrap();
        assert!(turn.content.contains("391"), "Expected arithmetic result");
        assert!(turn.calls.is_empty());
        if effort == ThinkingEffort::Off {
            assert_eq!(stats.0, 0)
        } else {
            assert!(stats.0 > 0, "Installed Qwen should emit supported thinking")
        }
        println!(
            "{}",
            json!({"mode":format!("{effort:?}"),"wallSeconds":start.elapsed().as_secs_f64(),"reasoningBytes":stats.0,"usage":stats.1,"publicAnswer":turn.content})
        );
        assert!(!turn.message.to_string().contains("thinking"));
    }
}
