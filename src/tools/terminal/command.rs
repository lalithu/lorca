use crate::agent_types::*;
use crate::tools::Workspace;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::{mpsc, watch},
};

pub const OUTPUT_LIMIT: usize = 16 * 1024;
#[derive(Clone, Debug)]
pub struct CommandSpec {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub directory: String,
}
impl CommandSpec {
    pub fn validate(
        executable: &str,
        args: &[String],
        directory: &str,
        workspace: &Workspace,
    ) -> Result<Self> {
        workspace.directory(directory)?;
        // No shell, interpreter, user-supplied binary, cargo aliases, arbitrary config,
        // networking or destructive git commands. Project build scripts still run, hence
        // every invocation additionally requires approval AND an inherited OS sandbox.
        let binary = match executable {
            "cargo" => {
                let sub = args.first().map(String::as_str).unwrap_or("");
                let allowed = match sub {
                    "test" | "check" => args[1..].iter().all(|s| {
                        matches!(
                            s.as_str(),
                            "--all-targets"
                                | "--lib"
                                | "--bins"
                                | "--tests"
                                | "--quiet"
                                | "--offline"
                                | "--locked"
                        )
                    }),
                    "fmt" => args[1..] == ["--check"],
                    "clippy" => args[1..].iter().map(String::as_str).eq([
                        "--all-targets",
                        "--",
                        "-D",
                        "warnings",
                    ]),
                    _ => false,
                };
                if !allowed {
                    return Err("Allowed cargo actions: test/check (limited flags), fmt --check, clippy --all-targets -- -D warnings".into());
                }
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .ok_or("Missing HOME")?
                    .join(".cargo/bin/cargo")
            }
            _ => {
                return Err(
                    "Only bounded cargo verification commands are available in v0.4A".into(),
                );
            }
        };
        if !binary.is_file() {
            return Err("Approved executable is unavailable".into());
        }
        Ok(Self {
            executable: binary,
            args: args.to_vec(),
            directory: directory.into(),
        })
    }
    pub fn action(&self, workspace: &Workspace, seconds: u64) -> Value {
        json!({"executable":self.executable,"arguments":self.args,"workingDirectory":workspace.root.join(&self.directory),
            "timeoutSeconds":seconds,"outputLimitBytes":OUTPUT_LIMIT,
            "scope":"OS sandbox: workspace build outputs and private temp writes; toolchain/cache reads; loopback networking only. Project build scripts and tests execute."})
    }
}
struct ProcessGroup(u32);
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        // The child is its own process-group leader. Kill descendants as well, including
        // when a cancelled future drops before wait(). This never targets our own group.
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.0 as i32), libc::SIGKILL);
        }
    }
}
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn quote(path: &Path) -> Result<String> {
    let s = path.to_str().ok_or("Non-UTF8 sandbox path")?;
    Ok(format!(
        "\"{}\"",
        s.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}
fn sandbox(workspace: &Workspace, tmp: &Path, home: &Path, writable: bool) -> Result<String> {
    if !cfg!(target_os = "macos") || !Path::new("/usr/bin/sandbox-exec").is_file() {
        return Err(
            "Command execution requires the macOS OS sandbox; unavailable on this platform".into(),
        );
    }
    // sandbox-exec enforces this in every descendant, including project build scripts.
    // No broad home read, network access, credential access, or write outside scope.
    let ancestors = workspace
        .root
        .ancestors()
        .chain(tmp.ancestors())
        .chain(home.ancestors())
        .map(|p| quote(p).map(|q| format!("(literal {q})")))
        .collect::<Result<Vec<_>>>()?
        .join(" ");
    let root = quote(&workspace.root)?;
    let tmp = quote(tmp)?;
    let cargo = quote(&home.join(".cargo/registry"))?;
    let cargo_git = quote(&home.join(".cargo/git"))?;
    let bins = quote(&home.join(".cargo/bin"))?;
    let rustup = quote(&home.join(".rustup"))?;
    let git = quote(&workspace.root.join(".git"))?;
    let write_root = if writable {
        format!("(allow file-write* (subpath {root}))")
    } else {
        String::new()
    };
    Ok(format!(
        r#"(version 1)
(deny default)
(allow process-exec process-fork sysctl-read)
(allow signal (target self) (target children) (target same-sandbox))
(allow mach-lookup (global-name "com.apple.system.logger") (global-name "com.apple.system.opendirectoryd.libinfo") (global-name "com.apple.cfprefsd.daemon") (global-name "com.apple.cfprefsd.agent"))
(allow file-read-metadata)
(allow file-read* {ancestors} (subpath "/private/etc/ssl") (subpath "/System") (subpath "/usr") (subpath "/bin") (subpath "/sbin") (subpath "/Library/Developer") (subpath "/Applications/Xcode.app") (subpath "/private/var/db") (subpath "/dev") (subpath {root}) (subpath {tmp}) (subpath {cargo}) (subpath {cargo_git}) (subpath {bins}) (subpath {rustup}))
{write_root}
(allow file-write* (subpath {tmp}) (literal "/dev/null"))
(deny file-write* (subpath {git}) (regex #"/\.git(/|$)"))
(allow network* (local ip "localhost:*") (remote ip "localhost:*"))
"#
    ))
}
async fn drain<R: AsyncRead + Unpin>(
    mut reader: R,
    channel: &'static str,
    tx: mpsc::Sender<(&'static str, Vec<u8>)>,
) {
    let mut remaining = OUTPUT_LIMIT;
    let mut buffer = [0; 2048];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let kept = n.min(remaining);
                remaining -= kept;
                if kept > 0 && tx.send((channel, buffer[..kept].to_vec())).await.is_err() {
                    break;
                }
            }
        }
    }
}
pub async fn run(
    spec: &CommandSpec,
    workspace: &Workspace,
    seconds: u64,
    mut cancel: watch::Receiver<bool>,
    output: &mut (dyn FnMut(Value) + Send),
) -> Result<Value> {
    // Revalidate both the command and directory immediately before spawning.
    CommandSpec::validate("cargo", &spec.args, &spec.directory, workspace)?;
    run_sandboxed(spec, workspace, seconds, &mut cancel, None, output).await
}
pub async fn run_interactive(
    spec: &CommandSpec,
    workspace: &Workspace,
    seconds: u64,
    mut cancel: watch::Receiver<bool>,
    input: mpsc::Receiver<Option<String>>,
    output: &mut (dyn FnMut(Value) + Send),
) -> Result<Value> {
    CommandSpec::validate("cargo", &spec.args, &spec.directory, workspace)?;
    run_sandboxed(spec, workspace, seconds, &mut cancel, Some(input), output).await
}
pub async fn git_diff(workspace: &Workspace, cancel: watch::Receiver<bool>) -> Result<Value> {
    let git = workspace.root.join(".git");
    let metadata = std::fs::symlink_metadata(&git)
        .map_err(|_| "git_diff requires a .git directory inside the workspace")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(
            "External Git metadata (including worktrees) is not available to git_diff in v0.4A"
                .into(),
        );
    }
    let spec = CommandSpec {
        executable: "/usr/bin/git".into(),
        args: vec![
            "-c".into(),
            "core.fsmonitor=false".into(),
            "-c".into(),
            "core.hooksPath=/dev/null".into(),
            format!("--git-dir={}", git.display()),
            format!("--work-tree={}", workspace.root.display()),
            "--no-pager".into(),
            "diff".into(),
            "--no-ext-diff".into(),
            "--no-textconv".into(),
            "--".into(),
            ".".into(),
        ],
        directory: ".".into(),
    };
    run_sandboxed(&spec, workspace, 10, &mut cancel.clone(), None, &mut |_| {}).await
}
async fn run_sandboxed(
    spec: &CommandSpec,
    workspace: &Workspace,
    seconds: u64,
    cancel: &mut watch::Receiver<bool>,
    mut input: Option<mpsc::Receiver<Option<String>>>,
    output: &mut (dyn FnMut(Value) + Send),
) -> Result<Value> {
    if *cancel.borrow() {
        return Err("Cancelled".into());
    }
    workspace.directory(&spec.directory)?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("Missing HOME")?;
    let mut tmp = Temp(std::env::temp_dir().join(format!("lorca-command-{}", crate::types::id())));
    std::fs::create_dir(&tmp.0).map_err(|e| e.to_string())?;
    tmp.0 = tmp.0.canonicalize().map_err(|e| e.to_string())?;
    std::fs::create_dir(tmp.0.join("home")).map_err(|e| e.to_string())?;
    let profile = sandbox(
        workspace,
        &tmp.0,
        &home,
        spec.executable != Path::new("/usr/bin/git"),
    )?;
    let mut child = Command::new("/usr/bin/sandbox-exec");
    child
        .args(["-p", &profile])
        .arg(&spec.executable)
        .args(&spec.args)
        .current_dir(workspace.root.join(&spec.directory))
        .env_clear()
        .env(
            "PATH",
            format!(
                "{}:/usr/bin:/bin:/usr/sbin:/sbin",
                home.join(".cargo/bin").display()
            ),
        )
        .env("HOME", tmp.0.join("home"))
        .env("TMPDIR", &tmp.0)
        .env("CARGO_HOME", home.join(".cargo"))
        .env("RUSTUP_HOME", home.join(".rustup"))
        .env("LORCA_COMMAND_SANDBOX", "1")
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TERM_COLOR", "never")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    child.process_group(0);
    let mut child = child.spawn().map_err(|e| format!("Sandbox spawn: {e}"))?;
    let mut stdin = child.stdin.take();
    let _group = ProcessGroup(child.id().ok_or("Missing process id")?);
    let (tx, mut rx) = mpsc::channel(16);
    let stdout = tokio::spawn(drain(child.stdout.take().unwrap(), "stdout", tx.clone()));
    let stderr = tokio::spawn(drain(child.stderr.take().unwrap(), "stderr", tx));
    let timer = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(timer);
    let mut combined = String::new();
    let mut total = 0;
    let mut code = None;
    let mut pipes_closed = false;
    loop {
        if code.is_some() && pipes_closed {
            break;
        }
        tokio::select! {
            biased;
            _=cancel.changed()=> { let _=child.kill().await; stdout.abort(); stderr.abort(); return Err("Cancelled".into()); },
            _=&mut timer=> { let _=child.kill().await; stdout.abort(); stderr.abort(); return Err(format!("Command timed out after {seconds}s")); },
            item=async {input.as_mut().unwrap().recv().await}, if input.is_some()=> {
                match item {
                    Some(Some(text))=>if let Some(pipe)=stdin.as_mut(){
                        tokio::select!{biased;
                            _=cancel.changed()=>return Err("Cancelled".into()),
                            _=&mut timer=>return Err(format!("Command timed out after {seconds}s")),
                            r=pipe.write_all(text.as_bytes())=>r.map_err(|e|format!("Command stdin: {e}"))?,
                        }
                    },
                    Some(None)|None=>{stdin.take();input=None;}
                }
            },
            result=child.wait(), if code.is_none()=> { code=Some(result.map_err(|e| e.to_string())?.code().unwrap_or(-1)); },
            item=rx.recv(), if !pipes_closed=> match item {
                Some((channel, bytes))=> { total += bytes.len(); let available=OUTPUT_LIMIT.saturating_sub(combined.len()); let chunk=String::from_utf8_lossy(&bytes); let chunk=bounded(&chunk, available); if available>0 { output(json!({"stream":channel,"text":chunk})); combined.push_str(&chunk); } },
                None=> pipes_closed=true,
            }
        }
    }
    let _ = stdout.await;
    let _ = stderr.await;
    Ok(
        json!({"ok":code==Some(0),"exitCode":code,"output":bounded(&combined,OUTPUT_LIMIT),"truncated":total>=OUTPUT_LIMIT}),
    )
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    fn fixture(body: &str) -> Temp {
        let root =
            Temp(std::env::temp_dir().join(format!("lorca-command-test-{}", crate::types::id())));
        std::fs::create_dir(&root.0).unwrap();
        std::fs::create_dir(root.0.join("src")).unwrap();
        std::fs::write(
            root.0.join("Cargo.toml"),
            "[package]\nname=\"agent_command_fixture\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
        )
        .unwrap();
        std::fs::write(root.0.join("src/lib.rs"), body).unwrap();
        root
    }
    async fn command(w: &Workspace, seconds: u64, cancel: watch::Receiver<bool>) -> Result<Value> {
        let spec = CommandSpec::validate("cargo", &["test".into()], ".", w)?;
        run(&spec, w, seconds, cancel, &mut |_| {}).await
    }
    #[tokio::test]
    async fn real_subprocess_is_sandboxed_bounded_and_reports_exit_status() {
        if std::env::var_os("LORCA_COMMAND_SANDBOX").is_some() {
            return;
        }
        let root = fixture(
            r#"#[test] fn safety() {
            assert!(std::fs::read_to_string("/etc/passwd").is_err());
            assert!(std::fs::write("/private/tmp/lorca-outside-sandbox-forbidden", "bad").is_err());
            assert!(std::fs::write(".git/config", "bad").is_err());
            println!("{}", "a".repeat(40000));
        }"#,
        );
        std::fs::create_dir(root.0.join(".git")).unwrap();
        // A failing test emits captured stdout, exercising the bounded pipe drain.
        std::fs::write(
            root.0.join("src/lib.rs"),
            std::fs::read_to_string(root.0.join("src/lib.rs"))
                .unwrap()
                .replace("println!", "eprintln!")
                .replace("40000));", "40000)); panic!(\"expected failure\");"),
        )
        .unwrap();
        let w = Workspace::new(&root.0).unwrap();
        let (_tx, rx) = watch::channel(false);
        let value = command(&w, 30, rx).await.unwrap();
        assert_eq!(value["exitCode"], 101, "{value}");
        assert_eq!(value["truncated"], true, "{value}");
        assert!(value["output"].as_str().unwrap().len() <= OUTPUT_LIMIT + 30);
        assert!(!root.0.join(".git/config").exists());
    }
    #[tokio::test]
    async fn git_diff_is_read_only_and_ignores_executable_hooks() {
        if std::env::var_os("LORCA_COMMAND_SANDBOX").is_some() {
            return;
        }
        let root = fixture("before\n");
        for args in [vec!["init", "-q"], vec!["add", "src/lib.rs"]] {
            assert!(
                std::process::Command::new("/usr/bin/git")
                    .args(args)
                    .current_dir(&root.0)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        std::fs::write(root.0.join("src/lib.rs"), "after\n").unwrap();
        use std::io::Write;
        writeln!(
            std::fs::OpenOptions::new()
                .append(true)
                .open(root.0.join(".git/config"))
                .unwrap(),
            "[core]\nfsmonitor = !touch hook-ran"
        )
        .unwrap();
        let w = Workspace::new(&root.0).unwrap();
        let (_tx, rx) = watch::channel(false);
        let value = git_diff(&w, rx).await.unwrap();
        assert_eq!(value["ok"], true, "{value}");
        assert!(value["output"].as_str().unwrap().contains("+after"));
        assert!(!root.0.join("hook-ran").exists());
    }
    #[tokio::test]
    async fn command_timeout_and_cancel_kill_descendants() {
        if std::env::var_os("LORCA_COMMAND_SANDBOX").is_some() {
            return;
        }
        let root = fixture(
            "#[test] fn slow(){std::thread::sleep(std::time::Duration::from_secs(20)); std::fs::write(\"escaped\",\"bad\").unwrap();}",
        );
        let w = Workspace::new(&root.0).unwrap();
        let (_tx, rx) = watch::channel(false);
        let result = command(&w, 1, rx).await;
        assert!(result.unwrap_err().contains("timed out"));
        let (tx, rx) = watch::channel(false);
        let signal = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(700)).await;
            tx.send_replace(true);
        });
        let now = std::time::Instant::now();
        assert!(command(&w, 30, rx).await.unwrap_err().contains("Cancelled"));
        signal.await.unwrap();
        assert!(now.elapsed() < Duration::from_secs(5));
        assert!(!root.0.join("escaped").exists());
    }
}
