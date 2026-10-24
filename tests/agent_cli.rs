use axum::{
    Json, Router,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{path::PathBuf, process::Stdio, time::Duration};
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[tokio::test]
async fn cli_returns_without_waiting_for_stdin_eof() {
    let root = Temp(std::env::temp_dir().join(format!("lorca-agent-cli-{}", lorca::types::id())));
    std::fs::create_dir(&root.0).unwrap();
    let app=Router::new()
        .route("/api/generate",post(|Json(v):Json<Value>|async move{Json(json!({"done":true,"model":v["model"],"done_reason":"load"}))}))
        .route("/api/ps",get(||async{Json(json!({"models":[]}))}))
        .route("/api/chat",post(|Json(v):Json<Value>|async move{
            let has_tool=v["messages"].as_array().unwrap().iter().any(|v|v["role"]=="tool");
            Json(json!({"done":true,"message":if has_tool{json!({"role":"assistant","content":"Verified the directory listing."})}else{json!({"role":"assistant","tool_calls":[{"function":{"name":"list_directory","arguments":{"path":"."}}}]})}}))
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let mock = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let exe = env!("CARGO_BIN_EXE_lorca");
    let mut backend = tokio::process::Command::new(exe)
        .arg("serve")
        .env("LORCA_DATA_DIR", &root.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        while !root.0.join("runtime/endpoint.json").is_file() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let mut cli = tokio::process::Command::new(exe)
        .args(["agent", "--workspace"])
        .arg(&root.0)
        .args(["--host", &host, "Inspect directory"])
        .env("LORCA_DATA_DIR", &root.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    // Hold stdin open: completion must not wait for EOF or a final keystroke.
    let stdin = cli.stdin.take().unwrap();
    let output = tokio::time::timeout(Duration::from_secs(8), cli.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Verified the directory listing"));
    drop(stdin);
    backend.kill().await.unwrap();
    let _ = backend.wait().await;
    mock.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn terminal_sigint_does_not_kill_the_automatically_started_backend() {
    // ps cannot inspect other processes inside the runner's inherited sandbox.
    if std::env::var_os("LORCA_COMMAND_SANDBOX").is_some() {
        return;
    }
    let root =
        Temp(std::env::temp_dir().join(format!("lorca-agent-sigint-{}", lorca::types::id())));
    std::fs::create_dir(&root.0).unwrap();
    let ready = std::sync::Arc::new(tokio::sync::Notify::new());
    let notification = ready.clone();
    let app = Router::new().route(
        "/api/generate",
        post(move || {
            let ready = notification.clone();
            async move {
                ready.notify_one();
                tokio::time::sleep(Duration::from_secs(20)).await;
                Json(json!({"done":true,"model":"mock","done_reason":"load"}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let mock = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut cli = tokio::process::Command::new(env!("CARGO_BIN_EXE_lorca"));
    cli.args(["agent", "--workspace"])
        .arg(&root.0)
        .args(["--model", "mock", "--host", &host, "Inspect directory"])
        .env("LORCA_DATA_DIR", &root.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .process_group(0);
    let mut cli = cli.spawn().unwrap();
    let pid = cli.id().unwrap();
    let stdin = cli.stdin.take().unwrap();
    tokio::time::timeout(Duration::from_secs(8), ready.notified())
        .await
        .unwrap();
    let processes = std::process::Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,pgid="])
        .output()
        .unwrap();
    let backend = String::from_utf8_lossy(&processes.stdout)
        .lines()
        .find_map(|line| {
            let values: Vec<u32> = line
                .split_whitespace()
                .filter_map(|v| v.parse().ok())
                .collect();
            if values.len() == 3 && values[1] == pid {
                Some((values[0], values[2]))
            } else {
                None
            }
        })
        .expect("CLI-spawned backend");
    struct OwnedBackend(u32);
    impl Drop for OwnedBackend {
        fn drop(&mut self) {
            unsafe {
                libc::kill(self.0 as i32, libc::SIGKILL);
            }
        }
    }
    let _backend = OwnedBackend(backend.0);
    assert_ne!(
        backend.1, pid,
        "backend must not inherit the terminal foreground signal group"
    );
    unsafe {
        assert_eq!(libc::kill(-(pid as i32), libc::SIGINT), 0);
    }
    let output = tokio::time::timeout(Duration::from_secs(6), cli.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Task cancelled"));
    drop(stdin);
    let endpoint: lorca::transport::Endpoint =
        serde_json::from_slice(&std::fs::read(root.0.join("runtime/endpoint.json")).unwrap())
            .unwrap();
    assert!(
        reqwest::Client::new()
            .get(format!("{}/health", endpoint.url))
            .bearer_auth(endpoint.token)
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    mock.abort();
}
