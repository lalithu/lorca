use lorca::context::{Context, complete_groups};
use serde_json::json;
#[test]
fn budget_compaction_retains_objective_evidence_and_valid_groups() {
    let mut history = vec![
        json!({"role":"system","content":"Safety: permissions require approval."}),
        json!({"role":"user","content":"Original objective"}),
    ];
    for i in 0..20 {
        history.push(json!({"role":"assistant","content":"","tool_calls":[{"id":format!("call{i}"),"function":{"name":"read_file","arguments":{"path":"src/core/mod.rs"}}}]}));
        history.push(json!({"role":"tool","tool_call_id":format!("call{i}"),"tool_name":"read_file","content":json!({"ok":true,"content":"large observation ".repeat(500)}).to_string()}));
    }
    let archive = history.clone();
    let mut context = Context::default();
    let evidence = json!({"verifiedFiles":["src/core/mod.rs"],"completed":[{"path":"src/core/mod.rs","ok":true}],"unresolved":["cargo test failed"],"denied":["do not retry"]});
    assert!(
        context
            .fit(&mut history, &[], 16000, "Original objective", &evidence)
            .unwrap()
    );
    assert!(context.tokens(&history, &[]) < 16000 - 2048);
    assert_eq!(history[0], archive[0]);
    assert!(
        history[1]["content"]
            .as_str()
            .unwrap()
            .contains("cargo test failed")
    );
    let repaired = complete_groups(history.clone());
    assert_eq!(repaired, history);
    assert_eq!(archive.len(), 42);
    assert_eq!(context.compactions, 1);
    assert!(
        !context
            .fit(&mut history, &[], 16000, "Original objective", &evidence)
            .unwrap()
    );
}
#[test]
fn interrupted_groups_are_explicitly_unconfirmed_never_replayed() {
    let input = vec![
        json!({"role":"assistant","tool_calls":[{"id":"one","function":{"name":"apply_patch"}},{"id":"two","function":{"name":"run_command"}}]}),
        json!({"role":"tool","tool_call_id":"one","content":"{\"ok\":true}"}),
        json!({"role":"user","content":"What happened?"}),
    ];
    let output = complete_groups(input);
    assert_eq!(output.len(), 4);
    assert_eq!(output[2]["tool_call_id"], "two");
    assert!(
        output[2]["content"]
            .as_str()
            .unwrap()
            .contains("unconfirmed")
    );
}
#[test]
fn oversized_objective_fails_clearly_without_mutating_saved_input() {
    let mut history = vec![
        json!({"role":"system","content":"safety"}),
        json!({"role":"user","content":"x".repeat(100000)}),
    ];
    let before = history.clone();
    let error = Context::default()
        .fit(
            &mut history,
            &[],
            4096,
            &"objective".repeat(10000),
            &json!({"unresolved":"preserve"}),
        )
        .unwrap_err();
    assert!(error.contains("essential objective"));
    assert_eq!(history, before);
}
#[test]
fn images_are_not_summarized_into_invented_descriptions() {
    let image =
        json!({"role":"user","content":"Explain this screenshot","images":["actual-image-data"]});
    let mut history = vec![json!({"role":"system","content":"rules"}), image.clone()];
    for _ in 0..14 {
        history.push(json!({"role":"assistant","content":"old context ".repeat(900)}));
    }
    Context::default()
        .fit(
            &mut history,
            &[],
            20000,
            "Explain this screenshot",
            &json!({}),
        )
        .unwrap();
    assert!(history.iter().any(|m| m["images"] == image["images"]));
}

#[test]
fn oversized_executed_patch_folds_as_a_whole_without_losing_evidence() {
    let mut messages = vec![
        json!({"role":"system","content":"Require human approval."}),
        json!({"role":"user","content":"Implement feature"}),
        json!({"role":"assistant","tool_calls":[{"id":"patch","function":{"name":"apply_patch","arguments":{"path":"src/lib.rs","old_text":"old".repeat(10000),"new_text":"new".repeat(10000)}}}]}),
        json!({"role":"tool","tool_name":"apply_patch","tool_call_id":"patch","content":json!({"ok":true,"verified":true,"path":"src/lib.rs"}).to_string()}),
    ];
    let mut context = Context::default();
    assert!(
        context
            .fit(
                &mut messages,
                &[],
                12000,
                "Implement feature",
                &json!({"completed":["patch"]})
            )
            .unwrap()
    );
    assert!(context.tokens(&messages, &[]) < 10000);
    assert_eq!(messages, complete_groups(messages.clone()));
    assert!(
        messages[1]["content"]
            .as_str()
            .unwrap()
            .contains("src/lib.rs")
    );
    assert!(messages[1]["content"].as_str().unwrap().contains("true"));
}

#[test]
fn repeated_checkpoints_do_not_become_pinned_narrative_recursively() {
    let mut context = Context::default();
    let mut messages = vec![
        json!({"role":"system","content":"permissions"}),
        json!({"role":"user","content":"Inspect; do not write"}),
    ];
    for _ in 0..10 {
        messages.push(json!({"role":"assistant","content":"observation ".repeat(4000)}));
        context
            .fit(
                &mut messages,
                &[],
                8192,
                "Inspect; do not write",
                &json!({"unresolved":["cargo test failed"],"denied":["patch"]}),
            )
            .unwrap();
        assert_eq!(
            messages
                .iter()
                .filter(|m| m["content"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("Working checkpoint (historical data): ")))
                .count(),
            1
        );
        assert!(context.tokens(&messages, &[]) <= 6144);
        assert!(
            messages[1]["content"]
                .as_str()
                .unwrap()
                .contains("cargo test failed")
        );
    }
}

#[test]
fn calibration_uses_raw_basis_even_when_the_previous_estimate_was_scaled() {
    let messages = vec![json!({"role":"user","content":"representative prompt"})];
    let mut context = Context::default();
    let raw = context.tokens(&messages, &[]);
    context.confirm(raw, raw * 2);
    let scaled = context.tokens(&messages, &[]);
    assert_eq!(scaled, raw * 2);
    context.confirm(scaled, scaled);
    assert_eq!(context.tokens(&messages, &[]), scaled);
    assert_eq!(lorca::context::generation_reserve(8192), 2048);
    assert_eq!(lorca::context::generation_reserve(4096), 1024);
}
