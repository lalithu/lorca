use crate::agent_types::*;
use crate::tools::Workspace;
use serde::{Deserialize, Serialize};
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
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CommandSpec {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub directory: String,
    working_directory: PathBuf,
    binary_identity: Value,
    pub read_only: bool,
    pub network: bool,
    pub git_metadata: bool,
    build_only: bool,
    linked_git: Option<GitLocation>,
}
impl CommandSpec {
    pub fn validate(
        executable: &str,
        args: &[String],
        directory: &str,
        workspace: &Workspace,
    ) -> Result<Self> {
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
        Self::new(binary, args, directory, workspace)
    }
    pub fn general(
        executable: &str,
        args: &[String],
        directory: &str,
        workspace: &Workspace,
    ) -> Result<Self> {
        if executable.is_empty()
            || executable.chars().any(char::is_control)
            || args.iter().any(|arg| arg.contains('\0'))
        {
            return Err("Supply an executable and separate JSON arguments".into());
        }
        let directory = workspace.relative(directory)?;
        let cwd = workspace.command_directory(&directory)?;
        let name = Path::new(executable);
        let binary = if name.is_absolute() {
            if !executable_in_scope(name, workspace) {
                return Err(
                    "Executable is outside the workspace and supported system toolchain paths"
                        .into(),
                );
            }
            name.to_owned()
        } else if executable.contains('/') {
            cwd.join(Workspace::path(executable)?)
        } else {
            search_path().into_iter().map(|dir| dir.join(name))
                .find(|path| binary_identity(path).is_ok())
                .ok_or_else(|| format!("Executable {executable} was not found. Install it or supply its absolute path."))?
        };
        let target = binary
            .canonicalize()
            .map_err(|e| format!("Resolve executable: {e}"))?;
        if !executable_in_scope(&target, workspace) {
            return Err(
                "Executable resolves outside the workspace and supported toolchain paths".into(),
            );
        }
        Self::new(binary, args, &directory, workspace)
    }
    fn new(
        executable: PathBuf,
        args: &[String],
        directory: &str,
        workspace: &Workspace,
    ) -> Result<Self> {
        let working_directory = workspace.command_directory(directory)?;
        let binary_identity = binary_identity(&executable)?;
        Ok(Self {
            executable,
            args: args.to_vec(),
            directory: directory.into(),
            working_directory,
            binary_identity,
            read_only: false,
            network: false,
            git_metadata: false,
            build_only: false,
            linked_git: None,
        })
    }
    pub fn with_git_metadata(&mut self, workspace: &Workspace) -> Result<()> {
        self.git_metadata = true;
        if let Some(location) = git_location(workspace, &self.working_directory)?
            && !location.common.starts_with(&workspace.root)
        {
            self.linked_git = Some(location);
        }
        Ok(())
    }
    pub(crate) fn allow_build_outputs(&mut self) {
        self.build_only = true;
        self.read_only = true;
    }
    fn revalidate(&self, workspace: &Workspace) -> Result<()> {
        if workspace.command_directory(&self.directory)? != self.working_directory
            || binary_identity(&self.executable)? != self.binary_identity
        {
            return Err(
                "Executable or working directory changed since approval; propose the command again"
                    .into(),
            );
        }
        if let Some(expected) = &self.linked_git
            && (git_location(workspace, &self.working_directory)?.as_ref() != Some(expected)
                || expected.directory.canonicalize().ok().as_ref() != Some(&expected.directory)
                || git_link(&expected.directory.join("commondir"))? != "../.."
                || expected
                    .directory
                    .join(git_link(&expected.directory.join("gitdir"))?)
                    .canonicalize()
                    .ok()
                    .as_ref()
                    != Some(&expected.root.join(".git")))
        {
            return Err("Linked Git metadata does not match the approved worktree".into());
        }
        Ok(())
    }
    pub fn action(&self, _workspace: &Workspace, seconds: u64) -> Value {
        let files = if self.build_only {
            "workspace read-only except a new Cargo.lock; Cargo output uses temporary storage"
        } else if self.read_only {
            "workspace read-only"
        } else if self.git_metadata {
            "workspace and approved Git metadata writable"
        } else {
            "workspace writable, Git metadata protected"
        };
        let network = if self.network {
            "outbound networking allowed"
        } else {
            "outbound networking blocked"
        };
        let linked = self
            .linked_git
            .as_ref()
            .map(|git| format!(" Linked Git metadata: {}.", git.common.display()))
            .unwrap_or_default();
        json!({"executable":self.executable,"arguments":self.args,"workingDirectory":self.working_directory,
            "timeoutSeconds":seconds,"outputLimitBytes":OUTPUT_LIMIT,
            "readOnly":self.read_only,"network":self.network,"gitMetadata":self.git_metadata,
            "scope":format!("OS sandbox: {files}; {network}; private temporary home and toolchain reads. Scripts and build hooks can run.{linked}")})
    }
}

fn executable_in_scope(path: &Path, workspace: &Workspace) -> bool {
    path.starts_with(&workspace.root)
        || [
            "/opt/homebrew",
            "/usr",
            "/bin",
            "/sbin",
            "/System",
            "/Library/Developer",
            "/Applications/Xcode.app",
        ]
        .iter()
        .any(|root| path.starts_with(root))
        || std::env::var_os("HOME").is_some_and(|home| {
            let home = PathBuf::from(home);
            path.starts_with(home.join(".cargo/bin")) || path.starts_with(home.join(".rustup"))
        })
}

fn search_path() -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from("/opt/homebrew/bin"),
        "/usr/local/bin".into(),
        "/usr/bin".into(),
        "/bin".into(),
        "/usr/sbin".into(),
        "/sbin".into(),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        paths.insert(0, PathBuf::from(home).join(".cargo/bin"));
    }
    paths
}

fn binary_identity(path: &Path) -> Result<Value> {
    let meta =
        std::fs::metadata(path).map_err(|e| format!("Executable {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err("Executable must be a regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.mode() & 0o111 == 0 {
            return Err("File is not executable".into());
        }
        Ok(json!([
            meta.dev(),
            meta.ino(),
            meta.len(),
            meta.ctime(),
            meta.ctime_nsec()
        ]))
    }
    #[cfg(not(unix))]
    Ok(json!(meta.len()))
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
fn sandbox(workspace: &Workspace, tmp: &Path, home: &Path, spec: &CommandSpec) -> Result<String> {
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
    let write_root = if spec.build_only {
        let lock = spec.working_directory.join("Cargo.lock");
        let create_lock = if !lock.exists() {
            format!("(literal {})", quote(&lock)?)
        } else {
            String::new()
        };
        if create_lock.is_empty() {
            String::new()
        } else {
            format!("(allow file-write* {create_lock})")
        }
    } else if !spec.read_only {
        format!("(allow file-write* (subpath {root}))")
    } else {
        String::new()
    };
    let git_protection = if spec.git_metadata && !spec.read_only {
        String::new()
    } else {
        format!(r#"(deny file-write* (subpath {git}) (regex #"/\.git(/|$)"))"#)
    };
    let network = if spec.network {
        "(allow network-outbound)"
    } else if spec.build_only {
        "(allow network* (local ip \"localhost:*\") (remote ip \"localhost:*\"))"
    } else {
        ""
    };
    let linked_git = spec
        .linked_git
        .as_ref()
        .map(|git| -> Result<String> {
            let path = quote(&git.common)?;
            let mode = if spec.git_metadata && !spec.read_only {
                "file-read* file-write*"
            } else {
                "file-read*"
            };
            Ok(format!("(allow {mode} (subpath {path}))"))
        })
        .transpose()?
        .unwrap_or_default();
    Ok(format!(
        r#"(version 1)
(deny default)
(allow process-exec process-fork sysctl-read)
(allow signal (target self) (target children) (target same-sandbox))
(allow mach-lookup (global-name "com.apple.system.logger") (global-name "com.apple.system.opendirectoryd.libinfo") (global-name "com.apple.cfprefsd.daemon") (global-name "com.apple.cfprefsd.agent"))
(allow file-read-metadata)
(allow file-read* {ancestors} (subpath "/private/etc/ssl") (subpath "/System") (subpath "/usr") (subpath "/opt/homebrew") (subpath "/bin") (subpath "/sbin") (subpath "/Library/Developer") (subpath "/Applications/Xcode.app") (subpath "/private/var/db") (subpath "/dev") (subpath {root}) (subpath {tmp}) (subpath {cargo}) (subpath {cargo_git}) (subpath {bins}) (subpath {rustup}))
{write_root}
(allow file-write* (subpath {tmp}) (literal "/dev/null"))
{git_protection}
{network}
{linked_git}
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
    run_sandboxed(spec, workspace, seconds, &mut cancel, Some(input), output).await
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct GitLocation {
    root: PathBuf,
    directory: PathBuf,
    common: PathBuf,
}

fn git_location(workspace: &Workspace, cwd: &Path) -> Result<Option<GitLocation>> {
    for root in cwd
        .ancestors()
        .take_while(|dir| dir.starts_with(&workspace.root))
    {
        let marker = root.join(".git");
        let metadata = match std::fs::symlink_metadata(&marker) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(format!("Git metadata: {err}")),
        };
        if metadata.is_dir() {
            return Ok(Some(GitLocation {
                root: root.into(),
                directory: marker.clone(),
                common: marker,
            }));
        }
        if !metadata.is_file() {
            return Err("Unsupported .git marker; symlinks are not followed".into());
        }
        let link = git_link(&marker)?;
        let target = link
            .strip_prefix("gitdir: ")
            .ok_or("Invalid .git worktree marker")?;
        // The authorized workspace supplies the location. Inspect external metadata
        // only after the resulting scope has been approved.
        let mut directory = PathBuf::new();
        for part in root.join(target).components() {
            if part == std::path::Component::ParentDir {
                if !directory.pop() {
                    return Err("Invalid Git worktree path".into());
                }
            } else if part != std::path::Component::CurDir {
                directory.push(part);
            }
        }
        if cfg!(target_os = "macos")
            && (directory.starts_with("/var") || directory.starts_with("/tmp"))
        {
            directory = Path::new("/private").join(directory.strip_prefix("/").unwrap());
        }
        let worktrees = directory
            .parent()
            .filter(|path| path.ends_with("worktrees"))
            .ok_or("Expected standard Git worktree metadata")?;
        let common = worktrees
            .parent()
            .filter(|path| path.ends_with(".git"))
            .ok_or("Expected a standard Git common directory")?;
        return Ok(Some(GitLocation {
            root: root.into(),
            common: common.into(),
            directory,
        }));
    }
    Ok(None)
}

fn git_link(path: &Path) -> Result<String> {
    use std::io::Read;
    let metadata = std::fs::symlink_metadata(path).map_err(|e| format!("Git metadata: {e}"))?;
    if !metadata.is_file() || metadata.len() > 4096 {
        return Err("Expected a bounded regular Git link file".into());
    }
    let mut text = String::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(4097).read_to_string(&mut text))
        .map_err(|e| format!("Read Git link: {e}"))?;
    let value = text.trim_end_matches(['\r', '\n']);
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err("Invalid Git link path".into());
    }
    Ok(value.into())
}

pub fn prepare_git_diff(
    workspace: &Workspace,
    directory: &str,
    staged: bool,
) -> Result<(CommandSpec, bool)> {
    let directory = workspace.relative(directory)?;
    let cwd = workspace.command_directory(&directory)?;
    let git = git_location(workspace, &cwd)?.ok_or("No Git repository in the active workspace")?;
    let mut args = vec![
        "-c".into(),
        "core.fsmonitor=false".into(),
        "-c".into(),
        "core.hooksPath=/dev/null".into(),
        format!("--git-dir={}", git.directory.display()),
        format!("--work-tree={}", git.root.display()),
        "--no-pager".into(),
        "diff".into(),
        "--no-ext-diff".into(),
        "--no-textconv".into(),
    ];
    if staged {
        args.push("--cached".into());
    }
    args.extend(["--".into(), ".".into()]);
    let mut spec = CommandSpec::new("/usr/bin/git".into(), &args, &directory, workspace)?;
    spec.read_only = true;
    let approval = !git.common.starts_with(&workspace.root);
    if approval {
        spec.linked_git = Some(git);
    }
    Ok((spec, approval))
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
    spec.revalidate(workspace)?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("Missing HOME")?;
    let mut tmp = Temp(std::env::temp_dir().join(format!("lorca-command-{}", crate::types::id())));
    std::fs::create_dir(&tmp.0).map_err(|e| e.to_string())?;
    tmp.0 = tmp.0.canonicalize().map_err(|e| e.to_string())?;
    std::fs::create_dir(tmp.0.join("home")).map_err(|e| e.to_string())?;
    let profile = sandbox(workspace, &tmp.0, &home, spec)?;
    let mut child = Command::new("/usr/bin/sandbox-exec");
    child
        .args(["-p", &profile])
        .arg(&spec.executable)
        .args(&spec.args)
        .current_dir(&spec.working_directory)
        .env_clear()
        .env(
            "PATH",
            std::env::join_paths(search_path()).map_err(|e| e.to_string())?,
        )
        .env("HOME", tmp.0.join("home"))
        .env("TMPDIR", &tmp.0)
        .env("CARGO_HOME", home.join(".cargo"))
        .env("RUSTUP_HOME", home.join(".rustup"))
        .env("LORCA_COMMAND_SANDBOX", "1")
        .env(
            "CARGO_NET_OFFLINE",
            if spec.network { "false" } else { "true" },
        )
        .env("GIT_OPTIONAL_LOCKS", if spec.read_only { "0" } else { "1" })
        .env("GIT_TERMINAL_PROMPT", "0")
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
    if spec.build_only {
        child.env("CARGO_TARGET_DIR", tmp.0.join("target"));
    }
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
        json!({"ok":code==Some(0),"exitCode":code,"output":bounded(&combined,OUTPUT_LIMIT),"truncated":total>=OUTPUT_LIMIT,"executable":spec.executable,"arguments":spec.args,"workingDirectory":spec.working_directory}),
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
        let value = run(
            &prepare_git_diff(&w, ".", false).unwrap().0,
            &w,
            10,
            rx,
            &mut |_| {},
        )
        .await
        .unwrap();
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
