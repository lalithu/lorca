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
