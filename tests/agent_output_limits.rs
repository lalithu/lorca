//! Regression coverage for the observed output-limit failure.
//! Run: cargo test --test agent_output_limits -- --nocapture
//! These exercise the real HTTP provider, agent, approval gate and file tools;
//! the Ollama responses are deterministic fixtures, not live model inference.
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::State,
    response::Response,
    routing::{get, post},
};
use lorca::{agent::*, provider::agent::OllamaAgent};
use serde_json::{Value, json};
use std::{
    convert::Infallible,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

const TASK: &str =
    "Update index.html: add dark mode, a file explorer, and a background gradient toggle.";
const ORIGINAL: &str = "<!doctype html><html><body>Playground</body></html>";
const DARK: &str =
    "<!doctype html><html><body><button id=\"dark-mode\">Dark mode</button></body></html>";
const FINISHED: &str = "<!doctype html><html><body><button id=\"dark-mode\">Dark mode</button><nav id=\"file-explorer\"></nav><button id=\"gradient-toggle\">Gradient</button></body></html>";
const INTERRUPTED: &str = "llama-server returned invalid tool call arguments for \"apply_patch\": unexpected end of JSON input";

#[derive(Clone, Copy)]
enum Scenario {
    Normal,
    Recover,
    Exhaust,
    TruncatedCall(bool),
    HttpFailure(&'static str, u16, bool),
    HttpThenLength,
    HttpWithRead,
}
struct Server {
    scenario: Scenario,
    requests: Mutex<Vec<Value>>,
    interruptions: Mutex<usize>,
}
fn succeeded(messages: &Value, id: &str) -> bool {
    messages.as_array().unwrap().iter().any(|m| {
        m["role"] == "tool"
            && m["tool_call_id"] == id
            && serde_json::from_str::<Value>(m["content"].as_str().unwrap_or(""))
                .is_ok_and(|r| r["ok"] == true)
    })
}
fn patch(id: &str, old: &str, new: &str) -> Value {
    json!({"role":"assistant","content":"","tool_calls":[{"id":id,"function":{
        "name":"apply_patch","arguments":{"path":"index.html","old_text":old,"new_text":new}
    }}]})
}
async fn chat(State(server): State<Arc<Server>>, Json(request): Json<Value>) -> Response {
    // The bounded interpretation pass is separate from operational recovery.
    // Keep the same deliberately failing generation scenarios and assertions below.
    if request["tools"]
        .as_array()
        .is_some_and(|a| a.iter().any(|v| v["function"]["name"] == "interpret_task"))
    {
        return Response::new(Body::from(format!(
            "{}\n",
            json!({"message":{"role":"assistant","tool_calls":[{"id":"intent","function":{"name":"interpret_task","arguments":{"relation":"new","objective":"Update index.html with the requested dark mode, explorer and gradient features","outcomes":[{"kind":"edit","target":"index.html"}],"edit_targets":["index.html"],"forbidden_targets":[],"read_only":false,"commands_forbidden":false,"clarification":null}}}]},"done":true,"done_reason":"stop"})
        )));
    }
    assert_eq!(request["stream"], true);
    assert_eq!(request["think"], false);
    assert!(
        request["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["function"]["name"] == "apply_patch")
    );
    server.requests.lock().unwrap().push(request.clone());
    let budget = request["options"]["num_predict"].as_u64().unwrap();
    let messages = &request["messages"];
    let mut frames = if !succeeded(messages, "dark-mode") {
        vec![json!({"message":patch("dark-mode", ORIGINAL, DARK),"done":false})]
    } else if succeeded(messages, "remaining-features") {
        vec![
            json!({"message":{"role":"assistant","content":"Updated index.html with dark mode, the file explorer, and the gradient toggle."},"done":false}),
        ]
    } else {
        let mut interruptions = server.interruptions.lock().unwrap();
        if matches!(server.scenario, Scenario::HttpWithRead)
            && *interruptions == 1
            && !succeeded(messages, "read-after-error")
        {
            let frame = json!({"message":{"role":"assistant","content":"","tool_calls":[{
                "id":"read-after-error","function":{"name":"read_file","arguments":{"path":"index.html"}}
            }]},"done":true,"done_reason":"stop"});
            return Response::new(Body::from(format!("{frame}\n")));
        }
        let failure = match server.scenario {
            Scenario::HttpFailure(error, status, repeated) if repeated || *interruptions == 0 => {
                Some((error, status))
            }
            Scenario::HttpThenLength if *interruptions == 0 => Some((INTERRUPTED, 500)),
            Scenario::HttpWithRead => Some((INTERRUPTED, 500)),
            _ => None,
        };
        if let Some((error, status)) = failure {
            *interruptions += 1;
            // Even a complete-looking call included in a rejected response must
            // never reach the approval gate or execute.
            return Response::builder()
                .status(status)
                .body(Body::from(
                    json!({
                        "error":error,"message":patch("cut-off-patch", DARK, "unacknowledged edit")
                    })
                    .to_string(),
                ))
                .unwrap();
        }
        let truncate = match server.scenario {
            Scenario::Normal | Scenario::HttpFailure(..) | Scenario::HttpWithRead => false,
            Scenario::Recover => *interruptions == 0,
            Scenario::Exhaust | Scenario::TruncatedCall(_) | Scenario::HttpThenLength => true,
        };
        if truncate {
            *interruptions += 1;
            let mut frames = vec![
                json!({"message":{"role":"assistant","content":"Let me add the remaining features."},"done":false}),
            ];
            if let Scenario::TruncatedCall(partial) = server.scenario {
                // A length-stopped turn can contain a complete call followed by
                // another call cut in the middle of its JSON arguments. None of
                // that unacknowledged turn may enter approval or execution.
                frames.push(json!({"message":patch("cut-off-patch", DARK, "unacknowledged edit"),"done":false}));
                if partial {
                    frames.push(
                        json!({"message":{"tool_calls":[{"index":1,"id":"partial-patch","function":{
                    "name":"apply_patch","arguments":"{\"path\":\"index.html\",\"old_text\":"
                }}]},"done":false}),
                    );
                }
            }
            frames.push(
                json!({"done":true,"done_reason":"length","prompt_eval_count":8300,
                "eval_count":budget,"eval_duration":1_000_000}),
            );
            frames
        } else {
            vec![json!({"message":patch("remaining-features", DARK, FINISHED),"done":false})]
        }
    };
    if frames.last().unwrap()["done"] != true {
        frames.push(
            json!({"done":true,"done_reason":"stop","prompt_eval_count":8300,
            "eval_count":150,"eval_duration":1_000_000}),
        );
    }
    // Exercise NDJSON transport framing as well as model-call JSON framing.
    let bytes = frames
        .into_iter()
        .map(|f| format!("{f}\n"))
        .collect::<String>()
        .into_bytes();
    let chunks = bytes
        .chunks(11)
        .map(|c| Ok::<_, Infallible>(Bytes::copy_from_slice(c)))
        .collect::<Vec<_>>();
    Response::new(Body::from_stream(futures_util::stream::iter(chunks)))
}
struct Fixture {
    root: PathBuf,
    server: Arc<Server>,
    http_task: tokio::task::JoinHandle<()>,
    agent: Arc<Agent>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.http_task.abort();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
async fn run(scenario: Scenario) -> (Fixture, TaskView) {
    run_with_approval(scenario, true).await
}
async fn run_with_approval(scenario: Scenario, allow_remaining: bool) -> (Fixture, TaskView) {
    let root = std::env::temp_dir().join(format!("lorca-output-limits-{}", lorca::types::id()));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("index.html"), ORIGINAL).unwrap();
    let server = Arc::new(Server {
        scenario,
        requests: Mutex::new(Vec::new()),
        interruptions: Mutex::new(0),
    });
    let router = Router::new()
        .route(
            "/api/ps",
            get(|| async {
                Json(json!({"models":[{"name":"test:latest","context_length":65536}]}))
            }),
        )
        .route("/api/chat", post(chat))
        .with_state(server.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let http_task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let agent = Arc::new(Agent::default());
    let id = agent
        .start(
            "output-limit-test",
            TASK,
            &root,
            Limits {
                iterations: 8,
                ..Default::default()
            },
            Arc::new(OllamaAgent {
                client: reqwest::Client::new(),
                host,
                model: "test".into(),
            }),
            Registry::default(),
        )
        .unwrap();
    let fixture = Fixture {
        root,
        server,
        http_task,
        agent,
    };
    let mut decisions = std::collections::HashSet::new();
    let view = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let view = fixture.agent.snapshot().unwrap();
            if let Some(approval) = &view.approval
                && decisions.insert(approval.id.clone())
            {
                // Only the two fully acknowledged fixture edits are permitted.
                let valid = (allow_remaining || approval.call_id != "remaining-features")
                    && matches!(
                        approval.call_id.as_str(),
                        "dark-mode" | "remaining-features"
                    );
                fixture
                    .agent
                    .decide(
                        "output-limit-test",
                        &id,
                        &approval.id,
                        if valid {
                            Decision::AllowOnce
                        } else {
                            Decision::Deny
                        },
                    )
                    .unwrap();
            }
            if !view.status.active() {
                break view;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await;
    if view.is_err() {
        let _ = fixture.agent.cancel("output-limit-test", &id);
    }
    (
        fixture,
        view.expect("Output-limit recovery must terminate within its bounded test budget"),
    )
}
fn no_generic_question_or_continuation(view: &TaskView) {
    assert!(
        !view
            .audit
            .iter()
            .any(|e| e.kind == "tool_requested" && e.data["tool"] == "request_user_input"),
        "An internal generation limit must not ask the user to clarify a clear request"
    );
    assert!(
        !view.audit.iter().any(|e| e.kind == "task_continued"),
        "Length exhaustion must use dedicated recovery, not the ordinary no-action continuation loop"
    );
}
fn assert_original_request_retained(fixture: &Fixture) {
    for request in fixture.server.requests.lock().unwrap().iter() {
        assert!(
            request["messages"].to_string().contains(TASK),
            "Recovery lost the active request"
        );
        assert!(
            request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["function"]["name"] == "apply_patch")
        );
    }
}
#[tokio::test]
async fn normal_completed_turns_apply_two_approved_edits_and_finish() {
    let (fixture, view) = run(Scenario::Normal).await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("index.html")).unwrap(),
        FINISHED
    );
    assert_eq!(
        view.audit
            .iter()
            .filter(|e| e.kind == "approval_required")
            .count(),
        2
    );
    assert_eq!(
        view.audit
            .iter()
            .filter(|e| e.kind == "file_changes")
            .count(),
        2
    );
    assert_eq!(fixture.server.requests.lock().unwrap().len(), 3);
    no_generic_question_or_continuation(&view);
    assert_original_request_retained(&fixture);
}
#[tokio::test]
async fn length_stop_recovers_without_generic_question_or_replaying_completed_edit() {
    let (fixture, view) = run(Scenario::Recover).await;
    no_generic_question_or_continuation(&view);
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("index.html")).unwrap(),
        FINISHED
    );
    assert_eq!(
        view.audit
            .iter()
            .filter(|e| e.kind == "approval_required")
            .count(),
        2
    );
    assert_eq!(
        view.audit
            .iter()
            .filter(|e| e.kind == "file_changes")
            .count(),
        2,
        "The completed dark-mode edit must not execute again during recovery"
    );
    assert!(
        view.audit.iter().any(|e| e.kind == "generation_truncated"),
        "Length-stop metadata must reach the agent audit"
    );
    let requests = fixture.server.requests.lock().unwrap();
    assert!(requests.len() <= 4, "Recovery must be bounded");
    assert!(
        requests[0]["options"]["num_predict"].as_u64().unwrap() > 2048,
        "The 65K context must not retain the accidental 2K output ceiling"
    );
    drop(requests);
    assert_original_request_retained(&fixture);
}
#[tokio::test]
async fn repeated_length_stops_report_generation_limit_without_asking_for_missing_instructions() {
    let (fixture, view) = run(Scenario::Exhaust).await;
    no_generic_question_or_continuation(&view);
    assert_eq!(view.status, Status::Incomplete, "{:?}", view.result);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("index.html")).unwrap(),
        DARK,
        "Previously verified work must survive exhausted recovery"
    );
    assert_eq!(
        view.audit
            .iter()
            .filter(|e| e.kind == "file_changes")
            .count(),
        1
    );
    let reason = view.result.as_deref().unwrap_or("").to_lowercase();
    assert!(
        reason.contains("generation")
            && (reason.contains("limit")
                || reason.contains("budget")
                || reason.contains("truncat")),
        "Report the actual generation blocker, not missing user intent: {reason}"
    );
    assert!(fixture.server.requests.lock().unwrap().len() <= 4);
    assert_original_request_retained(&fixture);
}
#[tokio::test]
async fn length_stopped_call_group_never_reaches_approval_or_execution() {
    for partial in [false, true] {
        let (fixture, view) = run(Scenario::TruncatedCall(partial)).await;
        assert!(
            !view.audit.iter().any(|e| matches!(
                e.kind.as_str(),
                "approval_required" | "tool_started"
            ) && e.data["callId"] == "cut-off-patch"),
            "A length-stopped call group reached approval/execution"
        );
        assert!(
            !view.audit.iter().any(|e| e.kind == "model_call_invalid"),
            "Output exhaustion must be recognized before parsing unfinished tool JSON"
        );
        no_generic_question_or_continuation(&view);
        assert_ne!(view.status, Status::Completed);
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("index.html")).unwrap(),
            DARK
        );
        assert_eq!(
            view.audit
                .iter()
                .filter(|e| e.kind == "file_changes")
                .count(),
            1
        );
        assert_original_request_retained(&fixture);
    }
}

#[tokio::test]
async fn malformed_tool_http_500_recovers_with_larger_budget_and_small_patch_instruction() {
    let (fixture, view) = run(Scenario::HttpFailure(INTERRUPTED, 500, false)).await;
    assert_eq!(view.status, Status::Completed, "{:?}", view.result);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("index.html")).unwrap(),
        FINISHED
    );
    no_generic_question_or_continuation(&view);
    assert_original_request_retained(&fixture);
    assert_eq!(
        view.audit
            .iter()
            .filter(|e| e.kind == "file_changes")
            .count(),
        2
    );
    assert_eq!(
        view.audit
            .iter()
            .filter(|e| e.kind == "approval_required")
            .count(),
        2
    );
    assert!(
        !view
            .audit
            .iter()
            .any(|e| e.data["callId"] == "cut-off-patch")
    );
    let interruption = view
        .audit
        .iter()
        .find(|e| e.kind == "tool_generation_interrupted")
        .unwrap();
    assert_eq!(interruption.data["executed"], false);
    assert_eq!(interruption.data["stopReason"], "incomplete_tool_arguments");
    assert!(
        interruption.data["generatedTokens"].is_null(),
        "Do not reuse the prior successful turn's usage as failed-turn evidence"
    );
    let requests = fixture.server.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(
        requests[2]["options"]["num_predict"].as_u64()
            > requests[1]["options"]["num_predict"].as_u64()
    );
    let retry = requests[2]["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap();
    assert!(
        retry.contains("small, targeted patches")
            && retry.contains("do not rewrite an entire file")
            && retry.contains("No tool calls")
    );
    assert_ne!(
        requests[1], requests[2],
        "Never replay the identical failing request"
    );
}

#[tokio::test]
async fn repeated_malformed_tool_http_500_stops_without_partial_execution_or_questions() {
    let (fixture, view) = run(Scenario::HttpFailure(INTERRUPTED, 500, true)).await;
    assert_eq!(view.status, Status::Incomplete, "{:?}", view.result);
    assert!(
        view.result
            .as_deref()
            .unwrap()
            .contains("incomplete tool-call JSON after bounded")
    );
    assert_eq!(fixture.server.requests.lock().unwrap().len(), 3);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("index.html")).unwrap(),
        DARK
    );
    assert_eq!(
        view.audit
            .iter()
            .filter(|e| e.kind == "approval_required")
            .count(),
        1
    );
    assert_eq!(
        view.audit
            .iter()
            .filter(|e| e.kind == "file_changes")
            .count(),
        1
    );
    assert!(
        !view
            .audit
            .iter()
            .any(|e| e.data["callId"] == "cut-off-patch")
    );
    no_generic_question_or_continuation(&view);
    assert_original_request_retained(&fixture);
}

#[tokio::test]
async fn unrelated_server_errors_and_non_500_errors_do_not_retry() {
    for (error, status) in [
        ("out of memory", 500),
        ("unexpected end of JSON input", 500),
        (
            "llama-server returned invalid tool call arguments for \"not_offered\": unexpected end of JSON input",
            500,
        ),
        (
            "llama-server returned invalid tool call arguments for \"apply_patch\": invalid character 'x'",
            500,
        ),
        (INTERRUPTED, 400),
    ] {
        let (fixture, view) = run(Scenario::HttpFailure(error, status, true)).await;
        assert_eq!(view.status, Status::Failed, "{:?}", view.result);
        assert!(view.result.as_deref().unwrap().contains(error));
        assert_eq!(
            fixture.server.requests.lock().unwrap().len(),
            2,
            "Must not retry {status}: {error}"
        );
        assert!(
            !view
                .audit
                .iter()
                .any(|e| e.kind == "tool_generation_interrupted")
        );
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("index.html")).unwrap(),
            DARK
        );
    }
}

#[tokio::test]
async fn mixed_http_interruption_and_length_stop_share_a_bounded_recovery() {
    let (fixture, view) = run(Scenario::HttpThenLength).await;
    assert_eq!(view.status, Status::Incomplete, "{:?}", view.result);
    assert_eq!(fixture.server.requests.lock().unwrap().len(), 3);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("index.html")).unwrap(),
        DARK
    );
    no_generic_question_or_continuation(&view);
}

#[tokio::test]
async fn successful_read_does_not_renew_malformed_tool_retry_allowance() {
    let (fixture, view) = run(Scenario::HttpWithRead).await;
    assert_eq!(view.status, Status::Incomplete, "{:?}", view.result);
    assert_eq!(fixture.server.requests.lock().unwrap().len(), 4);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("index.html")).unwrap(),
        DARK
    );
    assert_eq!(
        view.audit
            .iter()
            .filter(|e| e.kind == "tool_generation_interrupted")
            .count(),
        2
    );
    no_generic_question_or_continuation(&view);
}

#[tokio::test]
async fn recovered_patch_still_requires_approval_and_respects_denial() {
    let (fixture, view) =
        run_with_approval(Scenario::HttpFailure(INTERRUPTED, 500, false), false).await;
    assert_eq!(view.status, Status::Denied, "{:?}", view.result);
    assert_eq!(fixture.server.requests.lock().unwrap().len(), 3);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("index.html")).unwrap(),
        DARK
    );
    assert_eq!(
        view.audit
            .iter()
            .filter(|e| e.kind == "file_changes")
            .count(),
        1
    );
}
