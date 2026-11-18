use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use lorca::{core::Core, storage::Repository, types::id};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

struct Fake {
    requests: AtomicUsize,
    started: Notify,
    slow_first: bool,
    vision: std::sync::atomic::AtomicBool,
    image_count: AtomicUsize,
}
async fn chat(State(fake): State<Arc<Fake>>, Json(body): Json<Value>) -> String {
    assert!(body.get("tools").is_none());
    assert!(body["format"].is_object());
    assert_eq!(body["stream"], false);
    assert_eq!(body["think"], false);
    let number = fake.requests.fetch_add(1, Ordering::SeqCst);
    if number == 0 && fake.slow_first {
        fake.started.notify_one();
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let context: Value =
        serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
    let images = body["messages"][1]["images"].as_array().map_or(0, Vec::len);
    fake.image_count.fetch_add(images, Ordering::SeqCst);
    if context["imageObservations"]
        .as_str()
        .is_some_and(|s| !s.is_empty())
    {
        assert_eq!(
            body["format"]["properties"]["suggestions"]["items"]["properties"]["kind"]["const"],
            "caption"
        );
        let content = json!({"suggestions":[{"kind":"caption","title":"Study night","payload":{"text":"NLP study night, November 21, 2026 at 7pm. Free entry.","event":{"title":"NLP Study Night","dateIndex":0,"location":"NoDa Community Room"}}}]}).to_string();
        return json!({"message":{"role":"assistant","content":content},"done":true,"done_reason":"stop"}).to_string();
    }
    let content = json!({"suggestions":[{"kind":"priorities","source":context["note"],"title":"Tonight first","payload":{"text":"- B tonight\n- A tomorrow"}}]}).to_string();
    json!({"message":{"role":"assistant","content":content},"done":true,"done_reason":"stop"})
        .to_string()
        + "\n"
}
async fn fixture(
    slow_first: bool,
) -> (
    Arc<Core>,
    String,
    Arc<Fake>,
    tokio::task::JoinHandle<()>,
    std::path::PathBuf,
) {
    let fake = Arc::new(Fake {
        requests: AtomicUsize::new(0),
        started: Notify::new(),
        slow_first,
        vision: std::sync::atomic::AtomicBool::new(true),
        image_count: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route("/api/chat", post(chat))
        .route("/api/show", post(|State(fake): State<Arc<Fake>>| async move {
            Json(json!({"capabilities":if fake.vision.load(Ordering::SeqCst) { vec!["completion","vision"] } else { vec!["completion"] }}))
        }))
        .route(
            "/api/generate",
            post(|| async { Json(json!({"done":true,"done_reason":"load"})) }),
        )
        .route(
            "/api/ps",
            get(|| async { Json(json!({"models":[{"name":"orca","context_length":8192}]})) }),
        )
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let root = std::env::temp_dir().join(format!("lorca-notes-test-{}", id()));
    let core = Core::new(Repository::new(root.clone()).unwrap()).unwrap();
    let session = core
        .command(json!({"op":"connect","host":host,"model":"orca"}))
        .await
        .unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    (core, session, fake, server, root)
}
fn request(session: &str, revision: u64) -> Value {
    json!({"op":"notes_follow","session":session,"requestId":id(),"noteId":id(),"revision":revision,
        "text":"B tonight\nA tomorrow","localTime":"2026-11-18T12:00:00-05:00 America/New_York"})
}

#[tokio::test]
async fn typed_notes_request_returns_proposals_without_chat_or_tool_side_effects() {
    let (core, session, fake, server, root) = fixture(false).await;
    let before = serde_json::to_value(core.snapshot().conversations).unwrap();
    let req = request(&session, 7);
    let result = core.command(req.clone()).await.unwrap();
    assert_eq!(result["revision"], 7);
    assert_eq!(result["requestId"], req["requestId"]);
    assert_eq!(result["noteId"], req["noteId"]);
    assert_eq!(result["suggestions"][0]["kind"], "priorities");
    assert_eq!(fake.requests.load(Ordering::SeqCst), 1);
    assert_eq!(
        serde_json::to_value(core.snapshot().conversations).unwrap(),
        before
    );
    assert!(core.snapshot().turns.is_empty());
    assert!(
        core.command(json!({"op":"notes_follow","session":session,"text":"missing identifiers"}))
            .await
            .is_err()
    );
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn newer_revision_cancels_old_inference_and_owns_its_result() {
    let (core, session, fake, server, root) = fixture(true).await;
    let first = request(&session, 1);
    let clone = core.clone();
    let old = tokio::spawn(async move { clone.command(first).await });
    tokio::time::timeout(Duration::from_secs(3), fake.started.notified())
        .await
        .unwrap();
    let next = request(&session, 2);
    let result = core.command(next.clone()).await.unwrap();
    assert_eq!(result["requestId"], next["requestId"]);
    assert!(old.await.unwrap().unwrap_err().contains("cancelled"));
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn foreign_cancel_is_ignored_and_foreground_preparation_preempts_following() {
    let (core, session, fake, server, root) = fixture(true).await;
    let first = request(&session, 1);
    let req_id = first["requestId"].clone();
    let clone = core.clone();
    let old = tokio::spawn(async move { clone.command(first).await });
    tokio::time::timeout(Duration::from_secs(3), fake.started.notified())
        .await
        .unwrap();
    let other = core
        .command(json!({"op":"connect","model":"orca"}))
        .await
        .unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    core.command(json!({"op":"notes_cancel","session":other,"requestId":req_id}))
        .await
        .unwrap();
    assert!(!old.is_finished());
    core.command(json!({"op":"prepare","session":session,"model":"orca"}))
        .await
        .unwrap();
    assert!(old.await.unwrap().unwrap_err().contains("cancelled"));
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

fn image_request(session: &str) -> Value {
    use base64::Engine as _;
    let mut jpeg = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2)
        .write_to(&mut jpeg, image::ImageFormat::Jpeg)
        .unwrap();
    let mut req = request(session, 4);
    req["text"] = json!("Pasted image 1\n\u{fffc}");
    req["images"] = json!([base64::engine::general_purpose::STANDARD.encode(jpeg.into_inner())]);
    req["imageObservations"] = json!("NLP study night. November 21, 2026 at 7pm. Free entry.");
    req["dateCandidates"] = json!([{"id":0,"start":"2026-11-22T00:00:00Z","end":null,"timeZone":"America/New_York","display":"Nov 21, 2026 at 7pm"}]);
    req
}

#[tokio::test]
async fn image_caption_uses_vision_and_only_os_detected_calendar_dates() {
    let (core, session, fake, server, root) = fixture(false).await;
    let req = image_request(&session);
    let result = core.command(req.clone()).await.unwrap();
    assert_eq!(result["suggestions"][0]["kind"], "caption");
    assert_eq!(result["suggestions"][0]["source"], req["text"]);
    assert_eq!(
        result["suggestions"][0]["event"]["start"],
        "2026-11-22T00:00:00Z"
    );
    assert_eq!(result["suggestions"][0]["event"]["end"], Value::Null);
    assert_eq!(fake.image_count.load(Ordering::SeqCst), 1);
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn text_only_models_get_ocr_without_unsupported_image_inputs() {
    let (core, session, fake, server, root) = fixture(false).await;
    fake.vision.store(false, Ordering::SeqCst);
    let result = core.command(image_request(&session)).await.unwrap();
    assert_eq!(result["suggestions"][0]["kind"], "caption");
    assert_eq!(fake.image_count.load(Ordering::SeqCst), 0);
    assert_eq!(
        result["suggestions"][0]["event"]["start"],
        "2026-11-22T00:00:00Z"
    );
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn invalid_images_and_date_candidate_ids_are_rejected_before_inference() {
    let (core, session, fake, server, root) = fixture(false).await;
    let mut req = image_request(&session);
    req["images"] = json!(["not-an-image"]);
    assert!(core.command(req).await.is_err());
    let mut req = image_request(&session);
    req["dateCandidates"][0]["id"] = json!(7);
    assert!(core.command(req).await.is_err());
    assert_eq!(fake.requests.load(Ordering::SeqCst), 0);
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}
