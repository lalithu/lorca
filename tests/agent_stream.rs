use axum::{
    Json, Router,
    body::{Body, Bytes},
    response::Response,
    routing::{get, post},
};
use lorca::{
    agent::{Inference, ModelUpdate},
    provider::agent::{OllamaAgent, StreamTurn, parse_turn},
};
use serde_json::json;
use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};
async fn server(ack: bool) -> (String, tokio::task::JoinHandle<()>) {
    let app=Router::new().route("/api/ps",get(||async{Json(json!({"models":[{"name":"test:latest","context_length":8192}]}))}))
        .route("/api/chat",post(move|Json(request):Json<serde_json::Value>|async move{
            assert_eq!(request["stream"],true);assert_eq!(request["think"],false);
            let frames=vec![
                json!({"message":{"role":"assistant","content":"Hi 🐋"},"done":false}),
                json!({"message":{"tool_calls":[{"index":0,"id":"call","function":{"name":"read_","arguments":"{\"path\":"}}]},"done":false}),
                json!({"message":{"tool_calls":[{"index":0,"function":{"name":"file","arguments":"\"src/lib.rs\"}"}}]},"done":false}),
                json!({"message":{"content":" there"},"done":false}),
                json!({"done":true,"prompt_eval_count":70,"eval_count":10,"eval_duration":500_000_000})];
            let mut chunks=vec![];
            for frame in frames.into_iter().take(if ack{5}else{4}){
                let line=format!("{frame}\n").into_bytes();
                // Split every UTF-8 codepoint and JSON token at the transport boundary.
                for bytes in line.chunks(7){chunks.push(bytes.to_vec());}
            }
            let stream=futures_util::stream::unfold((chunks.into_iter(),0),|(mut chunks,n)|async move{
                let bytes=chunks.next()?;
                if n>0{tokio::time::sleep(Duration::from_millis(2)).await;}
                Some((Ok::<_,Infallible>(Bytes::from(bytes)),(chunks,n+1)))
            });
            Response::new(Body::from_stream(stream))
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (host, task)
}
#[tokio::test]
async fn real_incremental_text_and_fragmented_structured_calls_and_confirmed_usage() {
    let (host, server) = server(true).await;
    let model = OllamaAgent {
        client: reqwest::Client::new(),
        host,
        model: "test".into(),
    };
    let updates = Arc::new(Mutex::new(vec![]));
    let recorded = updates.clone();
    let task = tokio::spawn(async move {
        model
            .generate_stream(&[json!({"role":"user","content":"hi"})], &[], &|u| {
                recorded.lock().unwrap().push(u)
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while updates.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        !task.is_finished(),
        "Text must arrive before the structured turn finishes"
    );
    let turn = task.await.unwrap().unwrap();
    assert_eq!(turn.content, "Hi 🐋 there");
    assert_eq!(turn.calls[0].id, "call");
    assert_eq!(turn.calls[0].name, "read_file");
    assert_eq!(turn.calls[0].arguments["path"], "src/lib.rs");
    let values = updates.lock().unwrap();
    assert!(
        values
            .iter()
            .filter(|u| matches!(u, ModelUpdate::Text(_)))
            .count()
            >= 2
    );
    let usage = values
        .iter()
        .find_map(|u| {
            if let ModelUpdate::Usage(v) = u {
                Some(v)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(usage.context_limit, Some(8192));
    assert_eq!(usage.context_used, Some(80));
    assert_eq!(usage.prompt_tokens, Some(70));
    assert_eq!(usage.generated_tokens, Some(10));
    assert_eq!(usage.tokens_per_second, Some(20.0));
    server.abort();
}
#[tokio::test]
async fn interruption_preserves_partial_text_but_never_acknowledges_calls() {
    let (host, server) = server(false).await;
    let model = OllamaAgent {
        client: reqwest::Client::new(),
        host,
        model: "test".into(),
    };
    let output = Mutex::new(String::new());
    let result = model
        .generate_stream(&[], &[], &|u| {
            if let ModelUpdate::Text(s) = u {
                output.lock().unwrap().push_str(&s)
            }
        })
        .await;
    assert!(result.unwrap_err().contains("completion acknowledgement"));
    assert_eq!(*output.lock().unwrap(), "Hi 🐋 there");
    server.abort();
}
#[test]
fn malformed_calls_errors_and_private_thinking_are_not_executed_or_displayed() {
    let mut stream = StreamTurn::default();
    assert_eq!(
        stream
            .push(&json!({"message":{"thinking":"secret"}}))
            .unwrap(),
        ""
    );
    stream.push(&json!({"message":{"tool_calls":[{"index":0,"function":{"name":"apply_patch","arguments":"{bad"}}]}})).unwrap();
    assert!(parse_turn(stream.message().unwrap()).is_err());
    assert!(
        StreamTurn::default()
            .push(&json!({"error":"disconnected"}))
            .is_err()
    );
}

#[test]
fn eight_complete_unindexed_calls_keep_order_and_distinct_ids() {
    let mut stream = StreamTurn::default();
    let calls = (0..8)
        .map(|i| json!({"function":{"name":"read_file","arguments":{"path":format!("file{i}")}}}))
        .collect::<Vec<_>>();
    stream
        .push(&json!({"message":{"tool_calls":calls}}))
        .unwrap();
    let turn = parse_turn(stream.message().unwrap()).unwrap();
    assert_eq!(turn.calls.len(), 8);
    for (i, call) in turn.calls.iter().enumerate() {
        assert_eq!(call.arguments["path"], format!("file{i}"));
    }
}

async fn stop_server(
    reason: Option<&'static str>,
    tokens: u64,
    partial: bool,
    prompt_tokens: u64,
) -> (
    String,
    tokio::task::JoinHandle<()>,
    Arc<Mutex<Vec<serde_json::Value>>>,
) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let app = Router::new().route("/api/ps", get(|| async {Json(json!({"models":[{"name":"test:latest","context_length":65536}]}))})).route("/api/chat", post(move |Json(request): Json<serde_json::Value>| {
        let recorded = recorded.clone();
        async move {
            recorded.lock().unwrap().push(request);
            let args = if partial {json!("{\"path\":\"README.md\",")} else {json!({"path":"README.md"})};
            let mut done = json!({"done":true,"eval_count":tokens,"prompt_eval_count":prompt_tokens,"eval_duration":1_000_000});
            if let Some(reason) = reason {done["done_reason"] = json!(reason);}
            let frames = [json!({"message":{"role":"assistant","content":"Public progress only.","tool_calls":[{"id":"pending","function":{"name":"read_file","arguments":args}}]},"done":false}), done];
            Response::new(Body::from(frames.iter().map(|f|format!("{f}\n")).collect::<String>()))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (host, task, requests)
}
#[tokio::test]
async fn length_metadata_precedes_argument_parsing_and_preserves_public_progress_and_usage() {
    use lorca::{agent::FinishReason, context::GenerationBudget};
    for partial in [false, true] {
        let (host, server, requests) = stop_server(Some("length"), 2048, partial, 128).await;
        let model = OllamaAgent {
            client: reqwest::Client::new(),
            host,
            model: "test".into(),
        };
        let updates = Mutex::new(Vec::new());
        let budget = GenerationBudget {
            context_limit: 65536,
            output_limit: 2048,
            safety_margin: 1024,
        };
        let turn = model
            .generate_budgeted(
                &[json!({"role":"user","content":"Read README.md"})],
                &[],
                budget,
                &|u| updates.lock().unwrap().push(u),
            )
            .await
            .unwrap();
        assert_eq!(turn.finish_reason, FinishReason::Length);
        assert_eq!(turn.content, "Public progress only.");
        assert!(turn.calls.is_empty());
        assert!(turn.message.get("tool_calls").is_none());
        assert!(updates.lock().unwrap().iter().any(|u| matches!(u,ModelUpdate::Usage(v) if v.generated_tokens==Some(2048) && v.context_limit==Some(65536))));
        assert_eq!(requests.lock().unwrap()[0]["options"]["num_predict"], 2048);
        assert_eq!(requests.lock().unwrap()[0]["options"]["num_ctx"], 65536);
        server.abort();
    }
}
#[tokio::test]
async fn missing_stop_reason_stays_compatible_but_budget_exhaustion_is_conservative() {
    use lorca::{agent::FinishReason, context::GenerationBudget};
    for (reason, tokens, expected) in [
        (None, 100, FinishReason::Unknown),
        (None, 2048, FinishReason::Length),
        (Some("stop"), 2048, FinishReason::Stop),
    ] {
        let (host, server, _) = stop_server(reason, tokens, false, 128).await;
        let model = OllamaAgent {
            client: reqwest::Client::new(),
            host,
            model: "test".into(),
        };
        let budget = GenerationBudget {
            context_limit: 65536,
            output_limit: 2048,
            safety_margin: 1024,
        };
        let turn = model
            .generate_budgeted(&[], &[], budget, &|_| {})
            .await
            .unwrap();
        assert_eq!(turn.finish_reason, expected);
        assert_eq!(
            turn.calls.len(),
            usize::from(expected != FinishReason::Length)
        );
        server.abort();
    }
}
#[tokio::test]
async fn oversized_input_is_rejected_before_any_model_request() {
    use lorca::context::GenerationBudget;
    let (host, server, requests) = stop_server(Some("stop"), 10, false, 128).await;
    let model = OllamaAgent {
        client: reqwest::Client::new(),
        host,
        model: "test".into(),
    };
    let result = model
        .generate_budgeted(
            &[json!({"role":"user","content":"large input ".repeat(5000)})],
            &[],
            GenerationBudget::new(4096, false),
            &|_| {},
        )
        .await;
    assert!(result.unwrap_err().contains("Context cannot fit"));
    assert!(requests.lock().unwrap().is_empty());
    server.abort();
}

#[tokio::test]
async fn reported_input_cannot_silently_consume_reserved_output_space() {
    use lorca::context::GenerationBudget;
    let (host, server, requests) = stop_server(Some("stop"), 10, false, 64000).await;
    let model = OllamaAgent {
        client: reqwest::Client::new(),
        host,
        model: "test".into(),
    };
    let result = model
        .generate_budgeted(&[], &[], GenerationBudget::new(65536, false), &|_| {})
        .await;
    assert!(result.unwrap_err().contains("Context cannot fit"));
    assert_eq!(requests.lock().unwrap().len(), 1);
    server.abort();
}
