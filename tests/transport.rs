use lorca::{
    storage::{Repository, atomic},
    transport::{self, Endpoint},
    types::{Conversation, id},
};
use serde_json::{Value, json};
use std::{fs, time::Duration};
#[tokio::test]
async fn authenticated_stream_singleton_and_reconnection_preserve_state() {
    use futures_util::StreamExt;
    let root = std::env::temp_dir().join(format!("lorca-transport-test-{}", id()));
    let task = tokio::spawn(transport::serve(root.clone()));
    let path = root.join("runtime/endpoint.json");
    let endpoint = loop {
        if let Ok(bytes) = fs::read(&path) {
            break serde_json::from_slice::<Endpoint>(&bytes).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(endpoint.protocol, transport::PROTOCOL);
    let http = reqwest::Client::new();
    assert_eq!(
        http.post(format!("{}/command", endpoint.url))
            .json(&json!({"op":"connect"}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        http.post(format!("{}/command", endpoint.url))
            .bearer_auth(&endpoint.token)
            .header("Origin", "http://evil.example")
            .json(&json!({"op":"connect"}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    // A simultaneous launcher loses the file lock and cannot replace the endpoint.
    tokio::time::timeout(Duration::from_secs(1), transport::serve(root.clone()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fs::read(&path).unwrap(),
        serde_json::to_vec(&endpoint).unwrap()
    );
    let call = |v: Value| {
        http.post(format!("{}/command", endpoint.url))
            .bearer_auth(&endpoint.token)
            .json(&v)
            .send()
    };
    let connected: Value = call(json!({"op":"connect","model":"orca"}))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session = connected["session"].as_str().unwrap();
    let chat = connected["state"]["sessions"][0]["selectedId"]
        .as_str()
        .unwrap();
    let response = http
        .get(format!("{}/events", endpoint.url))
        .bearer_auth(&endpoint.token)
        .header("x-lorca-session", session)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let mut events = response.bytes_stream();
    let first = events.next().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&first).contains("conversations"));
    let updated = call(json!({"op":"draft","session":session,"text":"Reconnect me"}))
        .await
        .unwrap();
    assert_eq!(updated.status(), 200);
    let next = events.next().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&next).contains("Reconnect me"));
    drop(events);
    let snapshot: Value = call(json!({"op":"snapshot","session":session}))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(snapshot["conversations"][0]["draft"], "Reconnect me");
    call(json!({"op":"close","session":session,"unload":false}))
        .await
        .unwrap();
    task.abort();
    let _ = task.await;
    let repo = Repository::new(root.clone()).unwrap();
    assert_eq!(repo.load().unwrap().0[0].id, chat);
    assert_eq!(repo.load().unwrap().0[0].draft, "Reconnect me");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(root.join("runtime"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn deleting_chat_removes_only_unreferenced_images_and_skips_corrupt_chats() {
    let root = std::env::temp_dir().join(format!("lorca-delete-test-{}", id()));
    let repo = Repository::new(root.clone()).unwrap();
    let mut c = Conversation::default();
    let a = lorca::types::Attachment {
        id: id(),
        name: "image".into(),
        width: 1,
        height: 1,
    };
    c.draft_images.push(a.clone());
    repo.save(&c).unwrap();
    atomic(&repo.image_path(&a.id).unwrap(), b"test image").unwrap();
    repo.delete(&c.id).unwrap();
    repo.cleanup(std::slice::from_ref(&a)).unwrap();
    assert!(!repo.image_path(&a.id).unwrap().exists());
    atomic(&repo.image_path(&a.id).unwrap(), b"test image").unwrap();
    atomic(
        &root.join(format!("conversations/{}.json", id())),
        b"broken",
    )
    .unwrap();
    repo.cleanup(std::slice::from_ref(&a)).unwrap();
    assert!(repo.image_path(&a.id).unwrap().exists());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn agent_activity_stream_is_authenticated_and_reconnectable() {
    use futures_util::StreamExt;
    let root = std::env::temp_dir().join(format!("lorca-agent-transport-{}", id()));
    let server = tokio::spawn(transport::serve(root.clone()));
    let endpoint = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Ok(bytes) = fs::read(root.join("runtime/endpoint.json")) {
                break serde_json::from_slice::<Endpoint>(&bytes).unwrap();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let http = reqwest::Client::new();
    let url = format!("{}/agent/events", endpoint.url);
    assert_eq!(http.get(&url).send().await.unwrap().status(), 401);
    assert_eq!(
        http.get(&url)
            .bearer_auth(&endpoint.token)
            .header("x-lorca-session", "expired")
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let command = |value: Value| {
        http.post(format!("{}/command", endpoint.url))
            .bearer_auth(&endpoint.token)
            .json(&value)
            .send()
    };
    // An intentionally unavailable mock address; never contact the real Ollama instance.
    let v: Value = command(json!({"op":"connect","host":"http://127.0.0.1:1","model":"mock"}))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session = v["session"].as_str().unwrap();
    assert_eq!(
        http.get(&url)
            .bearer_auth(&endpoint.token)
            .header("Origin", "http://evil.example")
            .header("x-lorca-session", session)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let mut stream = http
        .get(&url)
        .bearer_auth(&endpoint.token)
        .header("x-lorca-session", session)
        .send()
        .await
        .unwrap()
        .bytes_stream();
    assert_eq!(
        serde_json::from_slice::<Value>(&stream.next().await.unwrap().unwrap()).unwrap(),
        Value::Null
    );
    let started: Value =
        command(json!({"op":"agent_start","session":session,"task":"Inspect","workspace":root}))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    let task = started["taskId"].as_str().unwrap();
    let failed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let bytes = stream.next().await.unwrap().unwrap();
            let v: Value = serde_json::from_slice(&bytes).unwrap();
            if v["status"] == "failed" {
                break v;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(failed["id"], task);
    assert_eq!(failed["events"][0]["kind"], "task_started");
    assert_eq!(
        failed["events"].as_array().unwrap().last().unwrap()["kind"],
        "task_failed"
    );
    drop(stream);
    let mut reconnected = http
        .get(&url)
        .bearer_auth(&endpoint.token)
        .header("x-lorca-session", session)
        .send()
        .await
        .unwrap()
        .bytes_stream();
    let snapshot: Value =
        serde_json::from_slice(&reconnected.next().await.unwrap().unwrap()).unwrap();
    assert_eq!(snapshot, failed);
    drop(reconnected);
    command(json!({"op":"close","session":session}))
        .await
        .unwrap();
    server.abort();
    let _ = server.await;
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn sigterm_stops_backend_and_preserves_existing_conversation_and_draft() {
    use std::process::{Command, Stdio};
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let root = std::env::temp_dir().join(format!("lorca-restart-test-{}", id()));
    let mut child = Child(
        Command::new(env!("CARGO_BIN_EXE_lorca"))
            .arg("serve")
            .env("LORCA_DATA_DIR", &root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let path = root.join("runtime/endpoint.json");
    let endpoint: Endpoint = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(bytes) = fs::read(&path) {
                break serde_json::from_slice(&bytes).unwrap();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let http = reqwest::Client::new();
    let health: Value = http
        .get(format!("{}/health", endpoint.url))
        .bearer_auth(&endpoint.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["pid"], child.0.id());
    let connected: Value = http
        .post(format!("{}/command", endpoint.url))
        .bearer_auth(&endpoint.token)
        .json(&json!({"op":"connect","model":"test"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session = connected["session"].as_str().unwrap();
    let saved = http
        .post(format!("{}/command", endpoint.url))
        .bearer_auth(&endpoint.token)
        .json(&json!({"op":"draft","session":session,"text":"Preserve me across rebuild"}))
        .send()
        .await
        .unwrap();
    assert!(saved.status().is_success());
    // Signal only the isolated subprocess, never the test runner or live backend.
    assert_eq!(unsafe { libc::kill(child.0.id() as i32, libc::SIGTERM) }, 0);
    let status = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(status.success(), "{status}");
    assert!(!path.exists());
    let repo = Repository::new(root.clone()).unwrap();
    assert!(
        repo.load()
            .unwrap()
            .0
            .iter()
            .any(|c| c.draft == "Preserve me across rebuild")
    );
    fs::remove_dir_all(root).unwrap();
}
