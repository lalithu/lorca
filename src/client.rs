use crate::{storage::default_root, transport::Endpoint};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{
    io,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};
#[derive(Clone)]
pub struct Client {
    pub endpoint: Endpoint,
    http: reqwest::Client,
}
impl Client {
    pub async fn connect(executable: &Path) -> io::Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(1))
            .build()
            .map_err(io::Error::other)?;
        let path = default_root().join("runtime/endpoint.json");
        let mut spawned = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        for _ in 0..100 {
            if std::time::Instant::now() > deadline {
                break;
            }
            if let Ok(bytes) = std::fs::read(&path)
                && let Ok(endpoint) = serde_json::from_slice::<Endpoint>(&bytes)
            {
                if endpoint.protocol != 1 {
                    return Err(io::Error::other(
                        "Incompatible lorca backend. Quit lorca clients and relaunch.",
                    ));
                }
                if !endpoint.url.starts_with("http://127.0.0.1:") {
                    return Err(io::Error::other("Invalid backend address"));
                }
                if http
                    .get(format!("{}/health", endpoint.url))
                    .bearer_auth(&endpoint.token)
                    .timeout(Duration::from_millis(200))
                    .send()
                    .await
                    .is_ok_and(|r| r.status().is_success())
                {
                    return Ok(Self { endpoint, http });
                }
            }
            if !spawned {
                let mut backend = Command::new(executable);
                backend
                    .arg("serve")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                // A terminal SIGINT belongs to its frontend, not the shared backend.
                #[cfg(unix)]
                {
                    use std::os::unix::process::CommandExt;
                    backend.process_group(0);
                }
                backend.spawn()?;
                spawned = true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Err(io::Error::other(
            "Could not start lorca’s Rust backend. Check Application Support permissions and relaunch.",
        ))
    }
    pub async fn command(&self, value: Value) -> Result<Value, String> {
        let timeout = match value["op"].as_str() {
            Some("prepare" | "close") => 140,
            Some("release") => 8,
            Some("heartbeat") => 2,
            _ => 5,
        };
        let response = self
            .http
            .post(format!("{}/command", self.endpoint.url))
            .bearer_auth(&self.endpoint.token)
            .json(&value)
            .timeout(Duration::from_secs(timeout))
            .send()
            .await
            .map_err(|_| {
                "Lost connection to lorca’s backend. Relaunch to reconnect.".to_string()
            })?;
        let status = response.status();
        let value: Value = response
            .json()
            .await
            .map_err(|_| "Unreadable backend response".to_string())?;
        if !status.is_success() {
            return Err(value["error"]
                .as_str()
                .unwrap_or("Backend request failed")
                .into());
        }
        Ok(value)
    }
    pub async fn events(
        &self,
        session: &str,
        send: tokio::sync::mpsc::UnboundedSender<Value>,
    ) -> Result<(), String> {
        let response = self
            .http
            .get(format!("{}/events", self.endpoint.url))
            .bearer_auth(&self.endpoint.token)
            .header("x-lorca-session", session)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        let mut bytes = response.bytes_stream();
        let mut pending = vec![];
        while let Some(chunk) = bytes.next().await {
            pending.extend_from_slice(&chunk.map_err(|e| e.to_string())?);
            while let Some(end) = pending.iter().position(|b| *b == b'\n') {
                let line: Vec<_> = pending.drain(..=end).collect();
                let value = serde_json::from_slice(&line).map_err(|e| e.to_string())?;
                send.send(value).map_err(|_| "Client closed".to_string())?;
            }
        }
        Err("Backend stream ended".into())
    }
    pub async fn attach(&self, model: &str, host: &str, fresh: bool) -> Result<String, String> {
        let result = self
            .command(json!({"op":"connect","model":model,"host":host,"fresh":fresh}))
            .await?;
        Ok(result["session"].as_str().ok_or("Missing session")?.into())
    }
}
