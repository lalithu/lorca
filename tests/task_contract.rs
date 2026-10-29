use lorca::agentloop::TaskContract;
use serde_json::json;
#[test]
fn contract_preserves_original_instruction_and_unknown_intent() {
    let text = "Inspect README.md; do not commit. Maybe improve it later.";
    let contract = TaskContract::new(text, Some("/workspace".into()), 3);
    assert_eq!(contract.objective.original_instruction, text);
    assert_eq!(contract.objective.requested_outcome, text);
    assert!(!contract.objective.restrictions.is_empty());
    assert!(contract.authority.requires_action_approval);
    assert_eq!(contract.context.conversation_messages, 3);
}
#[test]
fn conversations_need_no_execution_evidence() {
    let contract = TaskContract::new("Hello", None, 0);
    assert!(contract.objective.completion.report(&[]).satisfied);
}
#[test]
fn failed_command_is_evidence_of_execution_not_success() {
    let contract = TaskContract::new("Run cargo test", None, 0);
    let history = vec![
        json!({"role":"assistant","tool_calls":[{"id":"one","function":{"arguments":{"arguments":["test"]}}}]}),
        json!({"role":"tool","tool_name":"run_command","tool_call_id":"one","content":json!({"ok":false,"exitCode":1}).to_string()}),
    ];
    let report = contract.objective.completion.report(&history);
    assert!(report.satisfied); // The request is to run/report, not repair the failed test.
    assert_eq!(report.evidence[0]["exitCode"], 1);
}
