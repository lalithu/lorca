//! Opt-in diagnostic: records observed defects, never blesses them as desired behavior.
//! LORCA_BASELINE_OUTPUT=/absolute/result.json cargo test --test reliability_baseline -- --ignored --nocapture
use lorca::{
    agent::{Limits, Registry, ThinkingEffort, Workspace},
    agentloop::verification::Requirements,
};
use serde_json::{Value, json};

#[tokio::test]
#[ignore = "Checkpoint A diagnostic; writes only an explicitly selected report and disposable fixtures"]
async fn record_baseline_mechanics() {
    let output = std::env::var("LORCA_BASELINE_OUTPUT").expect("Set LORCA_BASELINE_OUTPUT");
    let root =
        std::env::temp_dir().join(format!("lorca-baseline-mechanics-{}", lorca::types::id()));
    std::fs::create_dir(&root).unwrap();
    let original = format!(
        "<!doctype html><html><body><main>{}</main></body></html>TAIL",
        "x".repeat(9000)
    );
    std::fs::write(root.join("index.html"), &original).unwrap();
    let workspace = Workspace::new(&root).unwrap();
    workspace.restore_references(&[json!({"role":"tool","tool_name":"list_directory","content":json!({"ok":true,"entries":[{"path":"index.html"}]}).to_string()})]);
    let registry = Registry::default();
    let limits = Limits::default();
    let (_, cancel) = tokio::sync::watch::channel(false);
    let read = registry.get("read_file").unwrap();
    let first = read
        .prepare(&workspace, json!({"path":"index.html"}), &limits)
        .unwrap();
    let first = read
        .execute(&workspace, &first, &limits, cancel.clone(), &mut |_| {})
        .await
        .unwrap();
    let second = read
        .prepare(
            &workspace,
            json!({"path":"index.html","offset":first["nextOffset"]}),
            &limits,
        )
        .unwrap();
    let second = read
        .execute(&workspace, &second, &limits, cancel.clone(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(first["truncated"], true);
    assert!(second["content"].as_str().unwrap().ends_with("TAIL"));
    let patch = registry.get("apply_patch").unwrap();
    let destructive = patch.prepare(
        &workspace,
        json!({"path":"index.html","old_text":first["content"],"new_text":""}),
        &limits,
    );
    let destructive = match destructive {
        Ok(p) => {
            json!({"prepared":true,"approvalRequired":p.approval,"executed":false,"remainingBytes":p.payload["replacement"].as_str().map(str::len)})
        }
        Err(e) => json!({"prepared":false,"error":e,"executed":false}),
    };
    let stale = patch
        .prepare(
            &workspace,
            json!({"path":"index.html","old_text":"TAIL","new_text":"END"}),
            &limits,
        )
        .unwrap();
    std::fs::write(root.join("index.html"), "human edit").unwrap();
    let stale_result = patch
        .execute(&workspace, &stale, &limits, cancel, &mut |_| {})
        .await;
    assert!(stale_result.is_err());
    assert_eq!(
        std::fs::read_to_string(root.join("index.html")).unwrap(),
        "human edit"
    );
    let tasks = [
        "Fix index.html",
        "Make the gradient control change hue across the rainbow",
        "The slider still doesn't work; fix it.",
        "still looks so bad and the slider still doesn’t work for the full rainbow; i dont understand why theres random black text in the middle of the page?? fix it",
        "Create an empty README.md",
        "Read index.html",
    ];
    let requirements: Vec<_> = tasks.iter().map(|task| json!({"task":task,"requirements":Requirements::new(task),"withoutEvidence":Requirements::new(task).report(&[])})).collect();
    let failed = vec![
        json!({"role":"assistant","tool_calls":[{"id":"cmd","function":{"name":"exec_command","arguments":{"executable":"cargo","arguments":["test"]}}}]}),
        json!({"role":"tool","tool_name":"exec_command","tool_call_id":"cmd","content":json!({"ok":false,"exitCode":101}).to_string()}),
    ];
    let commands: Vec<_> = [
        ("node", vec!["--check", "index.js"]),
        ("python3", vec!["-m", "unittest"]),
        ("cargo", vec!["test"]),
    ]
    .into_iter()
    .map(|(exe, args)| {
        let p = registry.get("exec_command").unwrap().prepare(
            &workspace,
            json!({"executable":exe,"arguments":args}),
            &limits,
        );
        json!({"executable":exe,"acceptedForApproval":p.is_ok(),"error":p.err()})
    })
    .collect();
    let timeouts: Vec<_> = [
        ThinkingEffort::Off,
        ThinkingEffort::Nominal,
        ThinkingEffort::High,
    ]
    .into_iter()
    .map(|effort| {
        let (budget, seconds) = lorca::context::GenerationBudget::new(65536, true).scheduled(
            120,
            &Default::default(),
            effort,
        );
        json!({"effort":effort,"requestedSeconds":120,"scheduledSeconds":seconds,"budget":budget})
    })
    .collect();
    let report: Value = json!({"kind":"diagnostic, not a product acceptance pass","requirements":requirements,"failedTestReporting":Requirements::new("Run cargo test and report the actual exit code and failures; do not edit files").report(&failed),"partialRead":{"first":first,"second":second},"destructivePrefix":destructive,"stalePatch":{"rejected":stale_result.is_err(),"humanContentPreserved":true},"commands":commands,"timeouts":timeouts,"tools":registry.metadata()});
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
