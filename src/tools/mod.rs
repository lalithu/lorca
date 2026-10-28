use command::CommandSpec;
mod command;
mod workspace;
pub use workspace::Workspace;

use crate::agent_types::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::watch;

pub struct Prepared {
    pub action: Value,
    pub approval: bool,
    pub payload: Value,
}
pub trait Tool: Send + Sync {
    fn schema(&self) -> Value;
    fn prepare(&self, workspace: &Workspace, arguments: Value, limits: &Limits)
    -> Result<Prepared>;
    fn execute<'a>(
        &'a self,
        workspace: &'a Workspace,
        prepared: &'a Prepared,
        limits: &'a Limits,
        cancel: watch::Receiver<bool>,
        output: &'a mut (dyn FnMut(Value) + Send),
    ) -> FutureResult<'a, Value>;
}
pub struct Registry(BTreeMap<String, Arc<dyn Tool>>);
impl Default for Registry {
    fn default() -> Self {
        let mut r = Self(BTreeMap::new());
        for kind in [
            "list_directory",
            "read_file",
            "search_files",
            "apply_patch",
            "run_command",
            "git_diff",
        ] {
            r.add(kind, Arc::new(Native(kind)));
        }
        r
    }
}
impl Registry {
    pub fn empty() -> Self {
        Self(BTreeMap::new())
    }
    pub fn add(&mut self, name: &str, tool: Arc<dyn Tool>) {
        self.0.insert(name.into(), tool);
    }
    pub fn get(&self, name: &str) -> Result<Arc<dyn Tool>> {
        self.0
            .get(name)
            .cloned()
            .ok_or_else(|| format!("Unknown tool: {name}"))
    }
    pub fn schemas(&self) -> Vec<Value> {
        self.0.values().map(|t| t.schema()).collect()
    }
}
struct Native(&'static str);
fn read_length() -> usize {
    8192
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathArgs {
    path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    path: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "read_length")]
    length: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    query: String,
    #[serde(default = "dot")]
    path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchArgs {
    path: String,
    old_text: String,
    new_text: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandArgs {
    executable: String,
    arguments: Vec<String>,
    #[serde(default = "dot")]
    directory: String,
}
fn dot() -> String {
    ".".into()
}
fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(v).map_err(|e| format!("Invalid arguments: {e}"))
}
fn string(v: &Value, key: &str) -> Result<String> {
    v[key]
        .as_str()
        .map(String::from)
        .ok_or_else(|| format!("Missing {key}"))
}
impl Tool for Native {
    fn schema(&self) -> Value {
        let (description, properties, required) = match self.0 {
            "list_directory" => (
                "List immediate entries, bounded to 200. Paths are relative to the workspace.",
                json!({"path":{"type":"string"}}),
                vec!["path"],
            ),
            "read_file" => (
                "Read a verified UTF-8 file in bounded excerpts (up to 8 KiB per call). Use nextOffset from a previous result to read a later excerpt when needed; don't repeat unchanged reads.",
                json!({"path":{"type":"string"},"offset":{"type":"integer","minimum":0,"description":"Optional UTF-8 byte offset; reuse nextOffset from the previous excerpt."},"length":{"type":"integer","minimum":4,"maximum":8192,"description":"Optional excerpt byte length, default 8192."}}),
                vec!["path"],
            ),
            "search_files" => (
                "Search filenames and UTF-8 contents for a literal query. Skips build and hidden folders; bounded scan.",
                json!({"query":{"type":"string","description":"Literal substring, NOT a regex. Example: reqwest"},"path":{"type":"string"}}),
                vec!["query"],
            ),
            "apply_patch" => (
                "Propose one atomic file edit. old_text must occur exactly once; empty old_text creates a new file only. Full diff requires user approval. Never delete files.",
                json!({"path":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"}}),
                vec!["path", "old_text", "new_text"],
            ),
            "run_command" => (
                "Request explicit approval for sandboxed cargo test/check, fmt --check, or clippy --all-targets -- -D warnings. No shell or arbitrary commands. Use executable cargo; arguments is an array.",
                json!({"executable":{"type":"string"},"arguments":{"type":"array","items":{"type":"string"}},"directory":{"type":"string"}}),
                vec!["executable", "arguments"],
            ),
            _ => (
                "Read tracked working-tree changes with git diff (bounded; no external diff/textconv).",
                json!({}),
                vec![],
            ),
        };
        json!({"type":"function","function":{"name":self.0,"description":description,"parameters":{"type":"object","properties":properties,"required":required,"additionalProperties":false}}})
    }
    fn prepare(&self, w: &Workspace, arguments: Value, limits: &Limits) -> Result<Prepared> {
        if serde_json::to_vec(&arguments)
            .map_err(|e| e.to_string())?
            .len()
            > 64 * 1024
        {
            return Err("Tool arguments exceed 64 KiB".into());
        }
        let mut action = arguments.clone();
        let mut payload = arguments;
        let approval = matches!(self.0, "apply_patch" | "run_command");
        match self.0 {
            "list_directory" => {
                let a: PathArgs = parse(payload.clone())?;
                Workspace::path(&a.path)?;
            }
            "read_file" => {
                let a: ReadArgs = parse(payload.clone())?;
                w.require_reference(&a.path)?;
                if !(4..=8192).contains(&a.length) {
                    return Err("Read length must be 4–8192 bytes".into());
                }
            }
            "search_files" => {
                let a: SearchArgs = parse(payload.clone())?;
                Workspace::path(&a.path)?;
                if a.query.is_empty() || a.query.len() > 200 {
                    return Err("Search query must contain 1–200 bytes".into());
                }
            }
            "apply_patch" => {
                let a: PatchArgs = parse(payload)?;
                let old = w.patch_source(&a.path)?;
                if old.is_some() {
                    w.require_reference(&a.path)?;
                } else {
                    let parent = std::path::Path::new(&a.path)
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or(std::path::Path::new("."));
                    if parent != std::path::Path::new(".") {
                        w.require_reference(&parent.to_string_lossy())?;
                    }
                }
                let new=match &old {
                    None if a.old_text.is_empty() => a.new_text,
                    Some(s) if !a.old_text.is_empty() && s.matches(&a.old_text).count()==1 => s.replacen(&a.old_text,&a.new_text,1),
                    _=>return Err("old_text must match exactly once; empty old_text is only for creating a new file".into()),
                };
                if new.len() > workspace::FILE_LIMIT || old.as_deref() == Some(&new) {
                    return Err("Patch is empty or too large".into());
                }
                let diff = full_diff(&a.path, old.as_deref(), &new);
                action = json!({"path":a.path,"diff":diff});
                payload = json!({"path":a.path,"expected":old,"replacement":new});
            }
            "run_command" => {
                let a: CommandArgs = parse(payload.clone())?;
                let spec = CommandSpec::validate(&a.executable, &a.arguments, &a.directory, w)?;
                action = spec.action(w, limits.command_seconds);
            }
            "git_diff" => {
                if payload != json!({}) {
                    return Err("git_diff takes no arguments".into());
                }
            }
            _ => return Err("Unknown native tool".into()),
        }
        Ok(Prepared {
            action,
            approval,
            payload,
        })
    }
    fn execute<'a>(
        &'a self,
        w: &'a Workspace,
        p: &'a Prepared,
        limits: &'a Limits,
        cancel: watch::Receiver<bool>,
        output: &'a mut (dyn FnMut(Value) + Send),
    ) -> FutureResult<'a, Value> {
        Box::pin(async move {
            if *cancel.borrow() {
                return Err("Cancelled".into());
            }
            match self.0 {
                "list_directory" => {
                    let a: PathArgs = parse(p.payload.clone())?;
                    let dir = w.directory(&a.path)?;
                    let mut entries = vec![];
                    let mut truncated = false;
                    for entry in dir.entries().map_err(|e| e.to_string())? {
                        if entries.len() == 200 {
                            truncated = true;
                            break;
                        }
                        let e = entry.map_err(|e| e.to_string())?;
                        let ty = e.file_type().map_err(|e| e.to_string())?;
                        let name = e.file_name().to_string_lossy().to_string();
                        let path = if a.path == "." {
                            name.clone()
                        } else {
                            format!("{}/{name}", a.path)
                        };
                        let reference = if ty.is_symlink() {
                            None
                        } else {
                            w.remember(&path).ok()
                        };
                        entries.push(json!({"name":name,"path":reference,"kind":if ty.is_dir(){"directory"}else if ty.is_symlink(){"symlink"}else{"file"}}));
                    }
                    entries.sort_by_key(|e| e["name"].as_str().unwrap_or("").to_string());
                    Ok(json!({"ok":true,"entries":entries,"truncated":truncated}))
                }
                "read_file" => {
                    let a: ReadArgs = parse(p.payload.clone())?;
                    w.excerpt(&a.path, a.offset, a.length)
                }
                "search_files" => {
                    let a: SearchArgs = parse(p.payload.clone())?;
                    search(w, &a, &cancel)
                }
                "apply_patch" => {
                    let path = string(&p.payload, "path")?;
                    let replacement = string(&p.payload, "replacement")?;
                    let expected: Option<String> = parse(p.payload["expected"].clone())?;
                    w.replace(&path, &expected, &replacement)?;
                    w.remember(&path)?;
                    Ok(json!({"ok":true,"verified":true,"path":path,"bytes":replacement.len()}))
                }
                "run_command" => {
                    let a: CommandArgs = parse(p.payload.clone())?;
                    let spec = CommandSpec::validate(&a.executable, &a.arguments, &a.directory, w)?;
                    command::run(&spec, w, limits.command_seconds, cancel, output).await
                }
                "git_diff" => command::git_diff(w, cancel).await,
                _ => Err("Unknown tool".into()),
            }
        })
    }
}
fn full_diff(path: &str, old: Option<&str>, new: &str) -> String {
    let mut diff = format!(
        "--- {}\n+++ {path}\n@@ -1,{} +1,{} @@\n",
        if old.is_some() { path } else { "/dev/null" },
        old.unwrap_or("").lines().count(),
        new.lines().count()
    );
    for (prefix, text) in [("-", old.unwrap_or("")), ("+", new)] {
        for line in text.split_inclusive('\n') {
            diff.push_str(prefix);
            diff.push_str(line);
            if !line.ends_with('\n') {
                diff.push_str("\n\\ No newline at end of file\n");
            }
        }
    }
    diff
}
fn search(w: &Workspace, a: &SearchArgs, cancel: &watch::Receiver<bool>) -> Result<Value> {
    let start = std::time::Instant::now();
    let mut pending = vec![a.path.clone()];
    let mut matches = vec![];
    let mut scanned = 0;
    let mut truncated = false;
    'scan: while let Some(path) = pending.pop() {
        if *cancel.borrow() {
            return Err("Cancelled".into());
        }
        let dir = w.directory(&path)?;
        for item in dir.entries().map_err(|e| e.to_string())? {
            if scanned >= 800 || matches.len() >= 30 || start.elapsed().as_secs() >= 2 {
                truncated = true;
                break 'scan;
            }
            scanned += 1;
            let entry = item.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || matches!(name.as_str(), "target" | "node_modules" | "build")
            {
                continue;
            }
            let p = if path == "." {
                name.clone()
            } else {
                format!("{path}/{name}")
            };
            let ty = entry.file_type().map_err(|e| e.to_string())?;
            if ty.is_symlink() {
                continue;
            }
            if name.contains(&a.query)
                && let Ok(reference) = w.remember(&p)
            {
                matches.push(json!({"path":reference}));
            }
            if ty.is_dir() {
                pending.push(p);
            } else if ty.is_file()
                && let Ok(text) = w.read(&p)
            {
                for (i, line) in text.lines().enumerate() {
                    if line.contains(&a.query) {
                        let reference = w.remember(&p)?;
                        matches.push(json!({"path":reference,"line":i+1,"text":bounded(line,240)}));
                        if matches.len() >= 30 {
                            truncated = true;
                            break 'scan;
                        }
                    }
                }
            }
        }
    }
    Ok(
        json!({"ok":true,"matches":matches,"scanned":scanned,"truncated":truncated,"note":"Literal substring search; regex operators are not interpreted"}),
    )
}
