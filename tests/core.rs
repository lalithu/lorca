use axum::{
    Json, Router,
    body::Body,
    extract::State,
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use lorca::{
    core::Core,
    storage::{Repository, atomic},
    types::*,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("lorca-core-test-{}", id()));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[derive(Default)]
struct Fake {
    requests: Mutex<Vec<Value>>,
    vision: Mutex<bool>,
    slow: Mutex<bool>,
    unconfirmed: Mutex<bool>,
}
async fn manage(
    State(fake): State<Arc<Fake>>,
    Json(v): Json<Value>,
) -> (axum::http::StatusCode, Json<Value>) {
    fake.requests.lock().unwrap().push(v.clone());
    if v["model"] == "slow" {
        tokio::time::sleep(Duration::from_secs(20)).await;
    }
    if v["keep_alive"] == 0 && *fake.unconfirmed.lock().unwrap() {
        return (
            axum::http::StatusCode::OK,
            Json(json!({"done":true,"done_reason":"load"})),
        );
    }
    if v["model"] == "broken" {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"not enough memory"})),
        );
    }
    (
        axum::http::StatusCode::OK,
        Json(
            json!({"model":v["model"],"done":true,"done_reason":if v["keep_alive"]==0{"unload"}else{"load"}}),
        ),
    )
}
async fn chat(State(fake): State<Arc<Fake>>, Json(v): Json<Value>) -> Body {
    fake.requests.lock().unwrap().push(v);
    let slow = *fake.slow.lock().unwrap();
    let stream = futures_util::stream::unfold(0, move |step| async move {
        match step {
            0 => Some((
                Ok::<_, std::io::Error>(
                    "{\"message\":{\"content\":\"Hello 🐋\"},\"done\":false}\n".to_string(),
                ),
                1,
            )),
            1 => {
                tokio::time::sleep(Duration::from_millis(if slow { 1500 } else { 30 })).await;
                Some((Ok("{\"message\":{\"content\":\" there\"},\"done\":true,\"prompt_eval_count\":64,\"eval_count\":8,\"eval_duration\":400000000}\n".to_string()),2))
            }
            _ => None,
        }
    });
    Body::from_stream(stream)
}
async fn mock() -> (String, Arc<Fake>, tokio::task::JoinHandle<()>) {
    let fake = Arc::new(Fake::default());
    *fake.vision.lock().unwrap() = true;
    let app=Router::new().route("/api/generate",post(manage)).route("/api/chat",post(chat))
        .route("/api/show",post(|State(f):State<Arc<Fake>>|async move{Json(json!({"capabilities":if *f.vision.lock().unwrap(){vec!["completion","vision"]}else{vec!["completion"]}}))}))
        .route("/api/tags",get(||async{Json(json!({"models":[{"name":"orca"},{"name":"second"}]}))}))
        .route("/api/ps",get(||async{Json(json!({"models":[{"name":"orca","context_length":4096},{"name":"second","context_length":4096}]}))})).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (host, fake, task)
}
async fn connect(core: &Arc<Core>, host: &str) -> String {
    core.command(json!({"op":"connect","host":host,"model":"orca"}))
        .await
        .unwrap()["session"]
        .as_str()
        .unwrap()
        .into()
}
async fn op(core: &Arc<Core>, session: &str, op: &str, fields: Value) -> Value {
    let mut v = fields;
    v["op"] = json!(op);
    v["session"] = json!(session);
    core.command(v).await.unwrap()
}
fn selected(core: &Core, session: &str) -> String {
    core.snapshot()
        .sessions
        .iter()
        .find(|s| s.id == session)
        .unwrap()
        .selected_id
        .clone()
}
async fn done(core: &Core) {
    for _ in 0..300 {
        if core.snapshot().turns.iter().all(|t| !t.running) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("Generation stuck");
}
fn jpeg() -> Vec<u8> {
    let mut out = std::io::Cursor::new(vec![]);
    image::RgbImage::from_pixel(10, 8, image::Rgb([255, 0, 0]))
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .unwrap();
    out.into_inner()
}

#[test]
fn foundation_compatibility_backup_is_idempotent_and_preserves_metadata() {
    let temp = Temp::new();
    let c = id();
    let a = id();
    let value = json!({"version":1,"id":c,"title":"Notes","createdAt":12345.25,"updatedAt":12346.5,"messages":[{"id":id(),"role":"user","content":"Look","attachments":[{"id":a,"name":"photo","width":10,"height":8}],"futureMessage":"keep"},{"id":id(),"role":"assistant","content":"Partial 🐋"}],"draft":"Next","draftImages":[],"futureMetadata":{"keep":true}});
    atomic(
        &temp.0.join(format!("conversations/{c}.json")),
        &serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    atomic(&temp.0.join(format!("attachments/{a}.jpg")), &jpeg()).unwrap();
    atomic(
        &temp.0.join("selected.json"),
        &serde_json::to_vec(&c).unwrap(),
    )
    .unwrap();
    let repo = Repository::new(temp.0.clone()).unwrap();
    let (chats, selection, warning) = repo.load().unwrap();
    assert!(warning.is_none());
    assert_eq!(selection, Some(c.clone()));
    assert_eq!(chats[0].created_at, 12345.25);
    assert_eq!(chats[0].messages[1].content, "Partial 🐋");
    repo.save(&chats[0]).unwrap();
    let original = fs::read(
        temp.0
            .join(format!("backups/pre-rust-core/conversations/{c}.json")),
    )
    .unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&original).unwrap(), value);
    Repository::new(temp.0.clone()).unwrap();
    assert_eq!(
        fs::read(
            temp.0
                .join(format!("backups/pre-rust-core/conversations/{c}.json"))
        )
        .unwrap(),
        original
    );
    assert_eq!(
        serde_json::from_slice::<Value>(
            &fs::read(temp.0.join(format!("conversations/{c}.json"))).unwrap()
        )
        .unwrap()["futureMetadata"],
        json!({"keep":true})
    );
    assert_eq!(fs::read(repo.image_path(&a).unwrap()).unwrap(), jpeg());
}
#[tokio::test]
async fn clients_share_crud_drafts_and_restore_without_stealing_selection() {
    let temp = Temp::new();
    let repo = Repository::new(temp.0.clone()).unwrap();
    let core = Core::new(repo.clone()).unwrap();
    let a = connect(&core, "http://127.0.0.1:1").await;
    let first = selected(&core, &a);
    op(&core, &a, "draft", json!({"text":"Draft survives"})).await;
    op(&core, &a, "rename", json!({"title":"  My notes  "})).await;
    let b = connect(&core, "http://127.0.0.1:1").await;
    assert_eq!(selected(&core, &b), first);
    op(&core, &a, "new", json!({})).await;
    let second = selected(&core, &a);
    assert_ne!(second, first);
    assert_eq!(selected(&core, &b), first);
    let restored = Core::new(repo).unwrap();
    assert_eq!(
        restored
            .snapshot()
            .conversations
            .iter()
            .find(|c| c.id == first)
            .unwrap()
            .draft,
        "Draft survives"
    );
    op(&core, &a, "delete", json!({"conversationId":first})).await;
    assert_eq!(selected(&core, &b), second);
    assert_eq!(core.snapshot().conversations.len(), 1);
    op(&core, &a, "delete", json!({"conversationId":second})).await;
    assert_eq!(core.snapshot().conversations.len(), 1);
}
#[test]
fn images_are_bounded_separate_and_cleanup_respects_shared_and_corrupt_metadata() {
    let temp = Temp::new();
    let repo = Repository::new(temp.0.clone()).unwrap();
    let shared = repo
        .import_image(&STANDARD.encode(jpeg()), "../../photo")
        .unwrap();
    assert!(repo.import_image(&STANDARD.encode(b"bad"), "fake").is_err());
    assert!(repo.image_path("../../escape").is_err());
    let mut first = Conversation::default();
    first.draft_images.push(shared.clone());
    repo.save(&first).unwrap();
    let mut second = Conversation::default();
    second.draft_images.push(shared.clone());
    repo.save(&second).unwrap();
    repo.delete(&first.id).unwrap();
    repo.cleanup(std::slice::from_ref(&shared)).unwrap();
    assert!(repo.image_path(&shared.id).unwrap().exists());
    atomic(
        &temp.0.join(format!("conversations/{}.json", id())),
        b"broken",
    )
    .unwrap();
    repo.delete(&second.id).unwrap();
    repo.cleanup(std::slice::from_ref(&shared)).unwrap();
    assert!(repo.image_path(&shared.id).unwrap().exists());
    assert!(repo.load().unwrap().2.is_some());
}
#[tokio::test]
async fn streaming_history_usage_and_duplicate_send_are_authoritative() {
    let temp = Temp::new();
    let (host, fake, server) = mock().await;
    let repo = Repository::new(temp.0.clone()).unwrap();
    let core = Core::new(repo.clone()).unwrap();
    let s = connect(&core, &host).await;
    let c = selected(&core, &s);
    let request = id();
    op(&core, &s, "draft", json!({"text":"Hi"})).await;
    let send = json!({"conversationId":c,"requestId":request});
    op(&core, &s, "send", send.clone()).await;
    op(&core, &s, "send", send.clone()).await;
    done(&core).await;
    assert_eq!(
        core.snapshot().conversations[0].messages[1].content,
        "Hello 🐋 there"
    );
    let usage = &core.snapshot().turns[0].usage;
    assert_eq!(usage.context_used, Some(72));
    assert_eq!(usage.context_limit, Some(4096));
    assert_eq!(usage.tokens_per_second, Some(20.0));
    op(
        &core,
        &s,
        "send",
        json!({"conversationId":c,"requestId":id(),"text":"Again"}),
    )
    .await;
    done(&core).await;
    let requests = fake.requests.lock().unwrap().clone();
    let chats: Vec<_> = requests
        .iter()
        .filter(|v| v["messages"].is_array())
        .collect();
    assert_eq!(chats.len(), 2);
    assert_eq!(chats[1]["messages"].as_array().unwrap().len(), 3);
    assert_eq!(chats[0]["keep_alive"], -1);
    assert!(core.command(json!({"op":"send","session":s,"conversationId":c,"requestId":request,"text":"duplicate"})).await.is_err());
    let restored = Core::new(repo).unwrap();
    assert_eq!(restored.snapshot().conversations[0].messages.len(), 4);
    server.abort();
}
#[tokio::test]
async fn cancellation_checkpoints_partial_and_never_leaks_to_another_chat() {
    let temp = Temp::new();
    let (host, fake, server) = mock().await;
    *fake.slow.lock().unwrap() = true;
    let repo = Repository::new(temp.0.clone()).unwrap();
    let core = Core::new(repo.clone()).unwrap();
    let s = connect(&core, &host).await;
    let c = selected(&core, &s);
    op(
        &core,
        &s,
        "send",
        json!({"conversationId":c,"requestId":id(),"text":"Slow"}),
    )
    .await;
    for _ in 0..200 {
        if core.snapshot().conversations[0]
            .messages
            .last()
            .unwrap()
            .content
            == "Hello 🐋"
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    core.maintenance();
    assert_eq!(
        repo.load().unwrap().0[0].messages.last().unwrap().content,
        "Hello 🐋"
    );
    op(&core, &s, "cancel", json!({"conversationId":c})).await;
    op(&core, &s, "new", json!({})).await;
    done(&core).await;
    assert!(
        core.snapshot()
            .conversations
            .iter()
            .find(|v| v.id == selected(&core, &s))
            .unwrap()
            .messages
            .is_empty()
    );
    assert_eq!(
        repo.load()
            .unwrap()
            .0
            .iter()
            .find(|v| v.id == c)
            .unwrap()
            .messages
            .last()
            .unwrap()
            .content,
        "Hello 🐋"
    );
    server.abort();
}
#[tokio::test]
async fn model_leases_prevent_unloading_another_session_and_switch_failure_restores() {
    let temp = Temp::new();
    let (host, fake, server) = mock().await;
    let core = Core::new(Repository::new(temp.0.clone()).unwrap()).unwrap();
    let a = connect(&core, &host).await;
    let b = connect(&core, &host).await;
    op(&core, &a, "prepare", json!({"model":"orca"})).await;
    op(&core, &b, "prepare", json!({"model":"orca"})).await;
    assert_eq!(fake.requests.lock().unwrap().len(), 1);
    assert!(
        core.command(json!({"op":"release","session":a,"model":"orca"}))
            .await
            .unwrap_err()
            .contains("another lorca session")
    );
    op(&core, &a, "close", json!({"unload":false})).await;
    op(
        &core,
        &b,
        "prepare",
        json!({"model":"broken","unloadPrevious":true}),
    )
    .await;
    let snap = core.snapshot();
    assert_eq!(snap.sessions[0].active_model.as_deref(), Some("orca"));
    assert!(
        snap.sessions[0]
            .error
            .as_ref()
            .unwrap()
            .contains("not enough memory")
    );
    let requests = fake.requests.lock().unwrap().clone();
    assert_eq!(requests[1]["keep_alive"], 0);
    assert_eq!(requests[2]["model"], "broken");
    assert_eq!(requests[3]["model"], "orca");
    let closed = op(&core, &b, "close", json!({"unload":true})).await;
    assert_eq!(closed["failures"], json!(["broken"]));
    server.abort();
}
#[tokio::test]
async fn vision_images_are_encoded_from_storage_and_unsupported_models_keep_messages() {
    let temp = Temp::new();
    let (host, fake, server) = mock().await;
    let core = Core::new(Repository::new(temp.0.clone()).unwrap()).unwrap();
    let s = connect(&core, &host).await;
    let c = selected(&core, &s);
    op(
        &core,
        &s,
        "importImage",
        json!({"name":"red.png","data":STANDARD.encode(jpeg())}),
    )
    .await;
    op(
        &core,
        &s,
        "send",
        json!({"conversationId":c,"requestId":id(),"text":"Color?"}),
    )
    .await;
    done(&core).await;
    let images = fake
        .requests
        .lock()
        .unwrap()
        .iter()
        .find(|v| v["messages"].is_array())
        .unwrap()["messages"][0]["images"]
        .clone();
    assert_eq!(
        STANDARD.decode(images[0].as_str().unwrap()).unwrap(),
        jpeg()
    );
    *fake.vision.lock().unwrap() = false;
    op(
        &core,
        &s,
        "send",
        json!({"conversationId":c,"requestId":id(),"text":"Again"}),
    )
    .await;
    done(&core).await;
    assert!(
        core.snapshot().turns[0]
            .error
            .as_ref()
            .unwrap()
            .contains("doesn’t support images")
    );
    assert_eq!(core.snapshot().conversations[0].messages.len(), 3);
    server.abort();
}
#[tokio::test]
async fn unavailable_ollama_is_an_error_without_stuck_generation() {
    let temp = Temp::new();
    let core = Core::new(Repository::new(temp.0.clone()).unwrap()).unwrap();
    let s = connect(&core, "http://127.0.0.1:1").await;
    let c = selected(&core, &s);
    op(
        &core,
        &s,
        "send",
        json!({"conversationId":c,"requestId":id(),"text":"Hello"}),
    )
    .await;
    done(&core).await;
    assert!(core.snapshot().turns[0].error.is_some());
    assert!(core.snapshot().sessions[0].loading_model.is_none());
    assert_eq!(core.snapshot().conversations[0].messages.len(), 1);
}

#[tokio::test]
async fn cancelled_model_loading_clears_busy_state_and_can_retry() {
    let temp = Temp::new();
    let (host, _fake, server) = mock().await;
    let core = Core::new(Repository::new(temp.0.clone()).unwrap()).unwrap();
    let s = connect(&core, &host).await;
    let c = core.clone();
    let sid = s.clone();
    let load = tokio::spawn(async move { op(&c, &sid, "prepare", json!({"model":"slow"})).await });
    for _ in 0..100 {
        if core.snapshot().sessions[0].loading_model.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    op(&core, &s, "cancelLoad", json!({})).await;
    tokio::time::timeout(Duration::from_secs(1), load)
        .await
        .unwrap()
        .unwrap();
    assert!(core.snapshot().sessions[0].loading_model.is_none());
    assert!(
        core.snapshot().sessions[0]
            .error
            .as_ref()
            .unwrap()
            .contains("cancelled")
    );
    op(&core, &s, "prepare", json!({"model":"orca"})).await;
    assert_eq!(
        core.snapshot().sessions[0].active_model.as_deref(),
        Some("orca")
    );
    server.abort();
}
#[tokio::test]
async fn unloading_requires_confirmation_and_failed_acknowledgement_retains_lease() {
    let temp = Temp::new();
    let (host, fake, server) = mock().await;
    let core = Core::new(Repository::new(temp.0.clone()).unwrap()).unwrap();
    let s = connect(&core, &host).await;
    op(&core, &s, "prepare", json!({"model":"orca"})).await;
    *fake.unconfirmed.lock().unwrap() = true;
    assert!(
        core.command(json!({"op":"release","session":s,"model":"orca"}))
            .await
            .unwrap_err()
            .contains("still reports")
    );
    assert_eq!(core.snapshot().sessions[0].models_to_release, vec!["orca"]);
    *fake.unconfirmed.lock().unwrap() = false;
    op(&core, &s, "release", json!({"model":"orca"})).await;
    assert!(core.snapshot().sessions[0].models_to_release.is_empty());
    server.abort();
}

#[tokio::test]
async fn an_explicit_stop_from_another_interface_cancels_the_shared_chat() {
    let temp = Temp::new();
    let (host, fake, server) = mock().await;
    *fake.slow.lock().unwrap() = true;
    let core = Core::new(Repository::new(temp.0.clone()).unwrap()).unwrap();
    let a = connect(&core, &host).await;
    let b = connect(&core, &host).await;
    let c = selected(&core, &a);
    op(
        &core,
        &a,
        "send",
        json!({"conversationId":c,"requestId":id(),"text":"Slow"}),
    )
    .await;
    for _ in 0..200 {
        if core.snapshot().conversations[0]
            .messages
            .last()
            .unwrap()
            .content
            == "Hello 🐋"
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    op(&core, &b, "cancel", json!({"conversationId":c})).await;
    assert!(core.snapshot().turns.iter().all(|t| !t.running));
    assert_eq!(
        core.snapshot().conversations[0]
            .messages
            .last()
            .unwrap()
            .content,
        "Hello 🐋"
    );
    server.abort();
}
#[tokio::test]
async fn image_limits_and_failed_writes_do_not_corrupt_saved_chats() {
    let temp = Temp::new();
    let repo = Repository::new(temp.0.clone()).unwrap();
    let core = Core::new(repo.clone()).unwrap();
    let s = connect(&core, "http://127.0.0.1:1").await;
    for _ in 0..4 {
        op(
            &core,
            &s,
            "importImage",
            json!({"name":"image","data":STANDARD.encode(jpeg())}),
        )
        .await;
    }
    assert!(
        core.command(
            json!({"op":"importImage","session":s,"name":"fifth","data":STANDARD.encode(jpeg())})
        )
        .await
        .unwrap_err()
        .contains("four")
    );
    let original = repo.load().unwrap().0[0].clone();
    let path = repo.root.join("conversations");
    fs::rename(&path, repo.root.join("saved")).unwrap();
    fs::write(&path, b"blocked").unwrap();
    assert!(
        core.command(json!({"op":"draft","session":s,"text":"Must not claim saved"}))
            .await
            .is_err()
    );
    assert_eq!(core.snapshot().conversations[0].draft, original.draft);
    fs::remove_file(path).unwrap();
    fs::rename(repo.root.join("saved"), repo.root.join("conversations")).unwrap();
    assert_eq!(repo.load().unwrap().0[0].draft_images.len(), 4);
}
