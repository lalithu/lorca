use super::*;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Mutex as AsyncMutex, watch},
};
struct Process {
    input: AsyncMutex<Option<tokio::process::ChildStdin>>,
    output: Arc<Mutex<VecDeque<u8>>>,
    exit: Mutex<Option<i32>>,
    cancel: watch::Sender<bool>,
    arguments: Vec<String>,
    observed: std::sync::atomic::AtomicBool,
    lost: Arc<Mutex<bool>>,
}
#[derive(Default)]
pub struct Terminal {
    sessions: Mutex<HashMap<String, Arc<Process>>>,
}
impl Drop for Terminal {
    fn drop(&mut self) {
        for p in self.sessions.lock().unwrap().values() {
            p.cancel.send_replace(true);
        }
    }
}
pub fn schemas() -> Vec<Value> {
    vec![
        schema(
            "exec_command",
            "Run an executable and argv after approval. arguments contains only arguments, never the executable itself; for example {\"executable\":\"pwd\",\"arguments\":[]}. This is HOST execution, not a sandbox. Use sh -c explicitly for shell syntax. Returns a session handle for longer commands; poll until exitCode is known. An omitted timeout means no total deadline; cancellation stops the process group.",
            json!({"executable":{"type":"string"},"arguments":{"type":"array","items":{"type":"string"}},"working_directory":{"type":"string"},"timeout_seconds":{"type":"integer"}}),
            &["executable", "arguments"],
        ),
        schema(
            "write_stdin",
            "Poll a running command; optionally send input or close stdin. Empty polling requires no new approval. Sending input requires approval.",
            json!({"session_id":{"type":"string"},"chars":{"type":"string"},"close_stdin":{"type":"boolean"},"yield_time_ms":{"type":"integer"}}),
            &["session_id"],
        ),
        schema(
            "git_diff",
            "Show a repository diff without modifying it.",
            json!({"path":{"type":"string"},"staged":{"type":"boolean"}}),
            &["path"],
        ),
    ]
}
pub fn prepare(call: &Call, c: &Context) -> Result<Prepared> {
    let a = &call.arguments;
    if call.name == "write_stdin" {
        let session = text(a, "session_id")?.to_owned();
        let chars = a["chars"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if chars.as_ref().is_some_and(|s| s.len() > 4096) {
            return Err("Input exceeds 4096 bytes".into());
        }
        let close = a["close_stdin"] == true;
        return Ok(Prepared {
            action: json!({"sessionId":session,"input":chars,"closeStdin":close}),
            access: if chars.is_some() || close {
                Access::Command
            } else {
                Access::None
            },
            operation: Operation::Poll {
                session,
                chars,
                close,
                wait: a["yield_time_ms"].as_u64().unwrap_or(1000).min(10000),
            },
        });
    }
    let dir = path(
        &c.cwd,
        a["working_directory"]
            .as_str()
            .or(a["path"].as_str())
            .unwrap_or("."),
    )?;
    if !dir.is_dir() {
        return Err("Working directory is not a directory".into());
    }
    let (exe, args, access) = if call.name == "git_diff" {
        (
            "/usr/bin/git".into(),
            vec![
                "--no-pager".into(),
                "diff".into(),
                "--no-ext-diff".into(),
                "--no-textconv".into(),
                if a["staged"] == true {
                    "--cached".into()
                } else {
                    "--".into()
                },
            ],
            Access::Command,
        )
    } else {
        let exe = text(a, "executable")?.to_owned();
        if exe.chars().any(char::is_control) {
            return Err("Invalid executable".into());
        }
        let args = a["arguments"]
            .as_array()
            .ok_or("Supply an argv array")?
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        (exe, args, Access::Command)
    };
    let timeout = a["timeout_seconds"].as_u64().map(|n| n.clamp(1, 86400));
    Ok(Prepared {
        action: json!({"executable":exe,"arguments":args,"workingDirectory":dir,"timeoutSeconds":timeout,"scope":"Host process with your user account's access; cancellation terminates its process group."}),
        access,
        operation: Operation::Command {
            executable: exe,
            arguments: args,
            directory: dir,
            timeout,
        },
    })
}
struct Group(u32);
impl Drop for Group {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.0 as i32), libc::SIGKILL);
        }
    }
}
impl Terminal {
    pub fn running(&self) -> bool {
        self.sessions
            .lock()
            .unwrap()
            .values()
            .any(|p| !p.observed.load(std::sync::atomic::Ordering::Relaxed))
    }
    pub async fn start(
        &self,
        exe: &str,
        args: &[String],
        dir: &Path,
        timeout: Option<u64>,
    ) -> Result<Value> {
        if self
            .sessions
            .lock()
            .unwrap()
            .values()
            .filter(|p| p.exit.lock().unwrap().is_none())
            .count()
            >= 4
        {
            return Err("Four commands are already running. Poll or cancel them first.".into());
        }
        let mut cmd = tokio::process::Command::new(exe);
        cmd.args(args)
            .current_dir(dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.as_std_mut().process_group(0);
        }
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let group = Group(child.id().ok_or("Missing process ID")?);
        let output = Arc::new(Mutex::new(VecDeque::new()));
        let lost = Arc::new(Mutex::new(false));
        let stdout = child.stdout.take().ok_or("Missing stdout")?;
        let stderr = child.stderr.take().ok_or("Missing stderr")?;
        let one = tokio::spawn(pump(stdout, output.clone(), lost.clone()));
        let two = tokio::spawn(pump(stderr, output.clone(), lost.clone()));
        let (tx, mut cancel) = watch::channel(false);
        let process = Arc::new(Process {
            input: AsyncMutex::new(child.stdin.take()),
            output,
            exit: Mutex::new(None),
            cancel: tx,
            arguments: args.to_vec(),
            observed: std::sync::atomic::AtomicBool::new(false),
            lost,
        });
        let session = crate::types::id();
        self.sessions
            .lock()
            .unwrap()
            .insert(session.clone(), process.clone());
        tokio::spawn(async move {
            let deadline = async {
                match timeout {
                    Some(s) => tokio::time::sleep(Duration::from_secs(s)).await,
                    None => std::future::pending::<()>().await,
                }
            };
            let result = tokio::select! {s=child.wait()=>s.ok().and_then(|s|s.code()).unwrap_or(-1),_=cancel.changed()=>{let _=child.kill().await;-1},_=deadline=>{let _=child.kill().await;-1}};
            drop(group);
            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                let _ = one.await;
                let _ = two.await;
            })
            .await;
            *process.exit.lock().unwrap() = Some(result);
        });
        self.poll(&session, None, false, 250).await
    }
    pub async fn poll(
        &self,
        id: &str,
        chars: Option<String>,
        close: bool,
        wait: u64,
    ) -> Result<Value> {
        let p = self
            .sessions
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or("Unknown command session")?;
        if chars.is_some() || close {
            let mut input = p.input.lock().await;
            if let Some(s) = chars {
                input
                    .as_mut()
                    .ok_or("stdin is closed")?
                    .write_all(s.as_bytes())
                    .await
                    .map_err(|e| e.to_string())?;
            }
            if close {
                input.take();
            }
        }
        let deadline = tokio::time::Instant::now() + Duration::from_millis(wait);
        while p.exit.lock().unwrap().is_none()
            && p.output.lock().unwrap().is_empty()
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let bytes: Vec<_> = {
            let mut out = p.output.lock().unwrap();
            let count = out.len().min(16384);
            out.drain(..count).collect()
        };
        let exit = *p.exit.lock().unwrap();
        if exit.is_some() {
            p.observed.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let buffered = !p.output.lock().unwrap().is_empty();
        let truncated = *p.lost.lock().unwrap();
        Ok(
            json!({"ok":exit.is_none_or(|n|n==0),"sessionId":id,"running":exit.is_none(),"exitCode":exit,"output":String::from_utf8_lossy(&bytes),"moreOutput":buffered,"outputTruncated":truncated,"arguments":p.arguments}),
        )
    }
}
async fn pump<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    out: Arc<Mutex<VecDeque<u8>>>,
    lost: Arc<Mutex<bool>>,
) {
    let mut b = [0; 4096];
    while let Ok(n) = reader.read(&mut b).await {
        if n == 0 {
            break;
        }
        let mut o = out.lock().unwrap();
        for byte in &b[..n] {
            if o.len() >= 65536 {
                o.pop_front();
                *lost.lock().unwrap() = true;
            }
            o.push_back(*byte);
        }
    }
}
