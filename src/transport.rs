use crate::{
    core::Core,
    storage::{Repository, atomic, default_root},
};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Endpoint {
    pub url: String,
    pub token: String,
    pub protocol: u32,
}
#[derive(Clone)]
struct Service {
    core: Arc<Core>,
    token: String,
}
fn authorized(headers: &HeaderMap, token: &str) -> bool {
    !headers.contains_key("origin")
        && headers.get("authorization").and_then(|v| v.to_str().ok())
            == Some(format!("Bearer {token}").as_str())
}
async fn command(
    State(service): State<Service>,
    headers: HeaderMap,
    Json(value): Json<Value>,
) -> Response {
    if !authorized(&headers, &service.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match service.core.command(value).await {
        Ok(v) => Json(v).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))).into_response(),
    }
}
async fn health(State(service): State<Service>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &service.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({"protocol":1})).into_response()
}
async fn events(State(service): State<Service>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &service.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(session) = headers
        .get("x-lorca-session")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
    else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if service.core.touch(&session).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let receiver = service.core.subscribe();
    let stream = futures_util::stream::unfold(
        (receiver, true, service.core, session),
        |(mut receiver, first, core, session)| async move {
            if !first {
                let _ = tokio::time::timeout(Duration::from_secs(10), receiver.changed()).await;
            }
            if core.touch(&session).is_err() {
                return None;
            }
            let mut bytes = serde_json::to_vec(&*receiver.borrow_and_update()).ok()?;
            bytes.push(b'\n');
            Some((
                Ok::<_, io::Error>(Bytes::from(bytes)),
                (receiver, false, core, session),
            ))
        },
    );
    (
        [
            ("content-type", "application/x-ndjson"),
            ("cache-control", "no-store"),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}
async fn agent_events(State(service): State<Service>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &service.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(session) = headers
        .get("x-lorca-session")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
    else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if service.core.touch(&session).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let receiver = service.core.agent().subscribe();
    let stream = futures_util::stream::unfold(
        (receiver, true, service.core, session),
        |(mut receiver, first, core, session)| async move {
            if !first {
                let _ = tokio::time::timeout(Duration::from_secs(10), receiver.changed()).await;
                tokio::time::sleep(Duration::from_millis(45)).await;
            }
            receiver.borrow_and_update();
            if core.touch(&session).is_err() {
                return None;
            }
            let mut bytes = serde_json::to_vec(&core.agent().snapshot()).ok()?;
            bytes.push(b'\n');
            Some((
                Ok::<_, io::Error>(Bytes::from(bytes)),
                (receiver, false, core, session),
            ))
        },
    );
    (
        [
            ("content-type", "application/x-ndjson"),
            ("cache-control", "no-store"),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}
pub async fn serve(root: PathBuf) -> io::Result<()> {
    let runtime = root.join("runtime");
    fs::create_dir_all(&runtime)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(runtime.join("backend.lock"))?;
    if fs2::FileExt::try_lock_exclusive(&lock).is_err() {
        return Ok(());
    }
    let core = Core::new(Repository::new(root)?).map_err(io::Error::other)?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let endpoint = Endpoint {
        url: format!("http://{}", listener.local_addr()?),
        token: token.clone(),
        protocol: 1,
    };
    atomic(
        &runtime.join("endpoint.json"),
        &serde_json::to_vec(&endpoint)?,
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            runtime.join("endpoint.json"),
            fs::Permissions::from_mode(0o600),
        )?;
    }
    let service = Service {
        core: core.clone(),
        token,
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/command", post(command))
        .route("/events", get(events))
        .route("/agent/events", get(agent_events))
        .layer(DefaultBodyLimit::max(14 * 1024 * 1024))
        .with_state(service);
    let shutdown = async move {
        let mut idle = std::time::Instant::now();
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            core.maintenance();
            if core.client_count() > 0 {
                idle = std::time::Instant::now();
            } else if idle.elapsed() > Duration::from_secs(30) {
                break;
            }
        }
    };
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await;
    let _ = fs::remove_file(runtime.join("endpoint.json"));
    drop(lock);
    result
}
pub fn run() -> io::Result<()> {
    tokio::runtime::Runtime::new()?.block_on(serve(default_root()))
}
