//! Deterministic stop checks over this task's real tool results, never model prose.
use serde::Serialize;
use serde_json::Value;

const EDIT_VERBS: &[&str] = &[
    "update",
    "updated",
    "edit",
    "modify",
    "change",
    "fix",
    "create",
    "write",
    "rewrite",
    "replace",
    "implement",
    "rename",
    "add",
    "remove",
    "improve",
    "revise",
    "correct",
    "repair",
    "refactor",
    "generate",
    "make",
];

#[derive(Clone, Debug, Serialize)]
pub struct Requirements {
    inspect: bool,
    directory_only: bool,
    edit: bool,
    paths: Vec<String>,
    empty_paths: Vec<String>,
    read_paths: Vec<String>,
    commands: Vec<String>,
    report_only: bool,
}
impl Requirements {
    pub fn new(task: &str) -> Self {
        let lower = task.to_lowercase();
        let explanatory = [
            "explain ",
            "what is ",
            "what did ",
            "what was ",
            "what changed",
            "how did ",
            "tell me what ",
            "how do ",
            "how to ",
            "describe how ",
        ]
        .iter()
        .any(|p| lower.starts_with(p));
        let words = |s: &str| {
            s.split(|c: char| !c.is_alphanumeric() && c != '_')
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        // Negative constraints are not action requests. Keep positive clauses,
        // including a later "run tests" after "do not edit README.md".
        let clauses = lower
            .replace(" and ", "\n")
            .replace(" then ", "\n")
            .replace("; ", "\n")
            .replace(". ", "\n");
        let positive = clauses
            .lines()
            .map(|clause| {
                let end = ["do not ", "don't ", "dont ", "never ", "without "]
                    .iter()
                    .filter_map(|marker| clause.find(marker))
                    .min()
                    .unwrap_or(clause.len());
                &clause[..end]
            })
            .collect::<Vec<_>>()
            .join(" and ");
        // A factual opening does not exempt a later explicit workspace action:
        // "Explain ownership and update README.md" still needs a verified edit.
        let explanatory = explanatory
            && !positive.split(" and ").skip(1).any(|clause| {
                let first = clause.split_whitespace().next().unwrap_or("");
                (EDIT_VERBS.contains(&first) && !requested_paths(clause).is_empty())
                    || clause.trim().starts_with("run cargo ")
            });
        let tokens = words(&positive);
        let has = |verbs: &[&str]| tokens.iter().any(|w| verbs.contains(&w.as_str()));
        let edit_requested = !explanatory && has(EDIT_VERBS);
        let mut paths = vec![];
        let mut read_paths = vec![];
        let mut editing = false;
        if !explanatory {
            // Keep input reads separate from edit targets. A bare coordinated
            // filename inherits the preceding edit: "update a.rs and b.rs".
            for clause in positive.split(" and ") {
                let tokens = words(clause);
                if tokens
                    .iter()
                    .any(|w| ["read", "inspect", "review", "examine"].contains(&w.as_str()))
                {
                    editing = false;
                    read_paths.extend(requested_read_paths(clause));
                } else if tokens.iter().any(|w| EDIT_VERBS.contains(&w.as_str())) {
                    editing = true;
                    paths.extend(requested_paths(clause));
                } else if tokens
                    .iter()
                    .any(|w| ["run", "execute", "rerun"].contains(&w.as_str()))
                {
                    editing = false;
                } else if editing {
                    paths.extend(requested_paths(clause));
                }
            }
        }
        paths = paths.into_iter().map(|p| normalized_path(&p)).collect();
        paths.sort();
        paths.dedup();
        read_paths.sort();
        read_paths.dedup();
        // Writing a poem or reading a greeting aloud is conversation, not an
        // implicit file mutation. Generic coding follow-ups retain action checks.
        let workspace_action = has(&[
            "file",
            "files",
            "repo",
            "repository",
            "workspace",
            "code",
            "bug",
            "function",
            "method",
            "class",
            "implementation",
            "readme",
            "source",
            "problem",
            "program",
            "script",
            "crate",
        ]);
        let edit = edit_requested
            && (!paths.is_empty()
                || workspace_action
                || has(&["fix", "refactor", "implement", "repair"]));
        let mut commands = vec![];
        if !explanatory
            && (has(&["run", "execute", "rerun"])
                || positive.split(" and ").any(|clause| {
                    let clause = clause.trim();
                    clause.starts_with("test ")
                        || clause.starts_with("check ")
                        || (clause.starts_with("verify ") && clause.contains("cargo "))
                }))
        {
            for name in ["test", "check", "clippy", "fmt"] {
                if positive.contains(&format!("cargo {name}"))
                    || (name == "test" && has(&["test", "tests"]))
                {
                    commands.push(name.into());
                }
            }
            if commands.is_empty() && has(&["run", "execute", "rerun"]) {
                commands.push(String::new());
            }
        }
        // Enforce only an explicit empty-file constraint for a single target.
        // "in an empty workspace" says nothing about the new file's contents.
        let plain = lower.replace(['`', '\"', '\''], "");
        let positive_plain = positive.replace(['`', '\"', '\''], "");
        let no_content = paths.first().is_some_and(|path| {
            ["do not populate", "dont populate"].iter().any(|prefix| {
                plain.match_indices(prefix).any(|(start, _)| {
                    let rest = plain[start + prefix.len()..]
                        .split([';', '\n'])
                        .next()
                        .unwrap_or("")
                        .trim()
                        .trim_end_matches('.');
                    rest.is_empty()
                        || rest == "it"
                        || rest == "the file"
                        || rest == path
                        || rest.starts_with("it or ")
                        || rest.starts_with("it and ")
                })
            })
        });
        let empty_paths = if edit
            && paths.len() == 1
            && (no_content
                || positive_plain.contains("empty file")
                || positive_plain.contains("empty content")
                || positive_plain.contains("leave it empty")
                || positive_plain.contains("leave it completely empty")
                || positive_plain.contains(&format!("empty {}", paths[0]))
                || positive_plain.contains(&format!("{} empty", paths[0])))
        {
            paths.clone()
        } else {
            Vec::new()
        };
        Self {
            inspect: !explanatory
                && has(&["inspect", "review", "read", "examine"])
                && (workspace_action || !read_paths.is_empty() || lower.contains("directory")),
            directory_only: lower.contains("directory")
                && !lower.contains("repo")
                && !lower.contains("project"),
            edit,
            paths,
            empty_paths,
            read_paths,
            report_only: !edit_requested,
            commands,
        }
    }
    pub(super) fn interpreted(task: &str, intent: &super::intent::Interpretation) -> Self {
        use super::intent::OutcomeKind::*;
        let mut value = Self::new(task);
        value.inspect |= intent.outcomes.iter().any(|o| o.kind == Inspect);
        value.edit |= intent
            .outcomes
            .iter()
            .any(|o| matches!(o.kind, Edit | RepairVerify));
        let semantic_paths: Vec<_> = intent
            .outcomes
            .iter()
            .filter(|o| matches!(o.kind, Edit | RepairVerify))
            .filter_map(|o| o.target.as_ref())
            .map(|p| normalized_path(p))
            .collect();
        value.paths.extend(semantic_paths);
        value.paths.sort();
        value.paths.dedup();
        value.read_paths.extend(
            intent
                .outcomes
                .iter()
                .filter(|o| o.kind == Inspect)
                .filter_map(|o| o.target.clone())
                // Abstract inspection subjects are not literal file references.
                // Keep concrete paths subject to the existing read-evidence check.
                .filter(|p| {
                    !matches!(
                        p.to_ascii_lowercase().as_str(),
                        "." | "repository" | "repo" | "workspace" | "project" | "directory"
                    )
                })
                .collect::<Vec<_>>(),
        );
        value.report_only = !value.edit && !intent.outcomes.iter().any(|o| o.kind == RepairVerify);
        if intent
            .outcomes
            .iter()
            .any(|o| matches!(o.kind, ExecuteReport | RepairVerify))
            && value.commands.is_empty()
        {
            value.commands.push(String::new());
        }
        if !intent
            .outcomes
            .iter()
            .any(|o| matches!(o.kind, ExecuteReport | RepairVerify))
        {
            // Do not erase a command explicitly required by the user.
        }
        value
    }
    pub(crate) fn requires_edit(&self) -> bool {
        self.edit
    }
    pub(super) fn reports_failure(&self) -> bool {
        self.report_only && !self.commands.is_empty()
    }
    pub fn action(&self) -> bool {
        self.inspect || self.edit || !self.commands.is_empty()
    }
    pub(super) fn validate_patch(&self, payload: &Value) -> Result<(), String> {
        let path = normalized_path(payload["path"].as_str().unwrap_or(""));
        if self.empty_paths.contains(&path) && payload["replacement"] != "" {
            return Err("Task constraint: the requested file must be empty. Use new_text as an empty string; nothing was executed.".into());
        }
        Ok(())
    }
    pub fn missing(&self, current: &[Value]) -> Vec<String> {
        let mut read = false;
        let mut read_paths = vec![];
        let mut patches = vec![];
        let mut patch_bytes = std::collections::HashMap::new();
        let mut commands = vec![];
        let mut last_edit = None;
        for (index, m) in current
            .iter()
            .enumerate()
            .filter(|(_, m)| m["role"] == "tool")
        {
            let Ok(mut result) = serde_json::from_str::<Value>(m["content"].as_str().unwrap_or(""))
            else {
                continue;
            };
            let already_executed = result["alreadyExecuted"] == true;
            if already_executed {
                result = result["evidence"].clone();
            }
            match m["tool_name"].as_str() {
                Some("list_directory")
                    if self.directory_only
                        && result["ok"] == true
                        && result["entries"].is_array() =>
                {
                    read = true
                }
                Some("read_file") if result["ok"] == true && result["content"].is_string() => {
                    read = true;
                    if let Some(path) = result["path"].as_str() {
                        read_paths.push(path.to_lowercase());
                    }
                }
                Some("apply_patch") if result["ok"] == true && result["verified"] == true => {
                    if let Some(path) = result["path"].as_str() {
                        patches.push(normalized_path(path));
                        patch_bytes.insert(normalized_path(path), result["bytes"].as_u64());
                        if !already_executed {
                            last_edit = Some(index);
                        }
                    }
                }
                Some("run_command" | "exec_command" | "write_stdin")
                    if result["exitCode"].is_number() =>
                {
                    if m["tool_name"] == "write_stdin" && result["arguments"].is_array() {
                        // A session begun before a patch is not fresh verification,
                        // even when its final poll arrives after that patch.
                        if let Some((started, _)) =
                            current[..index].iter().enumerate().find(|(_, m)| {
                                if m["tool_name"] != "exec_command" || m["role"] != "tool" {
                                    return false;
                                }
                                serde_json::from_str::<Value>(m["content"].as_str().unwrap_or(""))
                                    .is_ok_and(|v| {
                                        v["sessionId"].is_string()
                                            && v["sessionId"] == result["sessionId"]
                                    })
                            })
                        {
                            commands.push((
                                result["arguments"].clone(),
                                started,
                                result["exitCode"].as_i64(),
                            ));
                        }
                        continue;
                    }
                    let id = &m["tool_call_id"];
                    for call in current
                        .iter()
                        .filter_map(|m| m["tool_calls"].as_array())
                        .flatten()
                        .filter(|c| &c["id"] == id)
                    {
                        commands.push((
                            call["function"]["arguments"]["arguments"].clone(),
                            index,
                            result["exitCode"].as_i64(),
                        ));
                    }
                }
                _ => (),
            }
        }
        let mut missing = vec![];
        if self.inspect && !read {
            missing.push("Inspect relevant real files with read_file; a listing or a promise is not file inspection".into());
        }
        for path in &self.read_paths {
            if !read_satisfies(path, &read_paths) {
                missing.push(format!(
                    "Read the requested real file {path} with read_file"
                ));
            }
        }
        if self.edit {
            if self.paths.is_empty() && patches.is_empty() {
                missing.push("Apply and verify the requested file change with apply_patch".into());
            }
            for path in &self.paths {
                if !patches.contains(path) {
                    missing.push(format!(
                        "Apply and verify the requested change to {path} with apply_patch"
                    ));
                }
            }
        }
        for path in &self.empty_paths {
            if patch_bytes.get(path) != Some(&Some(0)) {
                missing.push(format!(
                    "Verify that {path} was created empty (zero bytes), as requested"
                ));
            }
        }
        for name in &self.commands {
            if !commands.iter().any(|(args, index, exit)| {
                if !self.report_only && *exit != Some(0) {
                    return false;
                }
                if self.edit && last_edit.is_some_and(|edit| *index <= edit) {
                    return false;
                }
                name.is_empty()
                    || args
                        .as_array()
                        .is_some_and(|a| a.first().is_some_and(|v| v == name))
            }) {
                missing.push(format!("Execute the requested cargo {name} using exec_command; report its actual exit status"));
            }
        }
        missing
    }
    /// A failed requested read is a concrete blocker, not a reason to keep
    /// asking the model to restate its completion summary.
    pub(super) fn read_blocker(&self, current: &[Value]) -> Option<String> {
        let results: Vec<Value> = current
            .iter()
            .filter(|m| m["role"] == "tool" && m["tool_name"] == "read_file")
            .filter_map(|m| serde_json::from_str(m["content"].as_str()?).ok())
            .collect();
        let successful: Vec<String> = results
            .iter()
            .filter(|v| v["ok"] == true && v["content"].is_string())
            .filter_map(|v| v["path"].as_str().map(str::to_lowercase))
            .collect();
        for requested in &self.read_paths {
            if read_satisfies(requested, &successful) {
                continue;
            }
            if let Some(error) = results
                .iter()
                .rev()
                .filter(|v| v["ok"] != true)
                .find_map(|v| {
                    let path = v["path"].as_str()?.to_lowercase();
                    (path == *requested || read_satisfies(requested, &[path]))
                        .then(|| v["error"].as_str())
                        .flatten()
                })
            {
                return Some(format!("Could not read {requested}: {error}"));
            }
        }
        if self.inspect
            && self.read_paths.is_empty()
            && successful.is_empty()
            && let Some(failure) = results.iter().rev().find(|v| v["ok"] != true)
            && let Some(error) = failure["error"].as_str()
        {
            return Some(format!("Could not inspect the requested file: {error}"));
        }
        None
    }
}

// Explicit paths stay exact. A bare filename may identify one unique
// successfully read workspace path; a same-name file elsewhere cannot satisfy
// a directory-qualified request. Duplicate observations are not ambiguity.
// Route a short standalone clarification through the existing question tool.
// This recognizes presentation, not task intent; it never authorizes an action.
pub(super) fn clarification_question(text: &str) -> Option<&str> {
    let text = text.trim();
    if text.len() > 2000 || !text.contains('?') {
        return None;
    }
    let question = text.rsplit_once(". ").map_or(text, |(_, tail)| tail).trim();
    let prefixes = [
        "what ",
        "which ",
        "where ",
        "how ",
        "could you ",
        "can you ",
        "would you ",
        "should ",
    ];
    let recognized = |s: &str| {
        prefixes
            .iter()
            .any(|p| s.trim().to_lowercase().starts_with(p))
    };
    if text.matches('?').count() == 1 && recognized(question) {
        return Some(question);
    }
    (recognized(text) || text.lines().any(recognized)).then_some(text)
}

fn normalized_path(path: &str) -> String {
    super::recovery::path_key(path)
        .unwrap_or_else(|| path.to_owned())
        .to_lowercase()
}

fn read_satisfies(requested: &str, observed: &[String]) -> bool {
    let Some(requested) = super::recovery::path_key(requested) else {
        return false;
    };
    let observed: std::collections::BTreeSet<_> = observed
        .iter()
        .filter_map(|p| super::recovery::path_key(p))
        .collect();
    if observed.contains(&requested) {
        return true;
    }
    !requested.contains('/')
        && observed
            .iter()
            .filter(|p| p.rsplit('/').next() == Some(requested.as_str()))
            .count()
            == 1
}
fn requested_read_paths(clause: &str) -> Vec<String> {
    let mut paths = requested_paths(clause);
    let words: Vec<_> = clause
        .split_whitespace()
        .map(|w| w.trim_matches(['`', '\"', '\'', ',', ';', ':']))
        .collect();
    let directories: std::collections::BTreeSet<_> = words
        .windows(2)
        .filter_map(|pair| {
            let dir = pair[1].trim_end_matches('/');
            (["in", "under", "inside"].contains(&pair[0])
                && dir.contains('/')
                && std::path::Path::new(dir).extension().is_none())
            .then(|| super::recovery::path_key(dir))
            .flatten()
        })
        .collect();
    if directories.len() == 1 {
        let dir = directories.first().unwrap();
        for path in &mut paths {
            if !path.contains('/') {
                *path = format!("{dir}/{path}");
            }
        }
    }
    paths
}

pub(super) fn requested_paths(clause: &str) -> Vec<String> {
    let end = [" to ", " from ", " based on ", " using ", " for "]
        .iter()
        .filter_map(|marker| clause.find(marker))
        .min()
        .unwrap_or(clause.len());
    clause[..end]
        .split_whitespace()
        .filter_map(|word| {
            let path = word
                .trim_matches(|c: char| !c.is_alphanumeric() && !['/', '_', '.', '-'].contains(&c))
                .trim_end_matches('.');
            let path = path.strip_prefix("./").unwrap_or(path);
            if ["license", "makefile", "dockerfile", "notice", "copying"].contains(&path) {
                return Some(path.into());
            }
            if path == "readme" {
                return Some("readme.md".into());
            }
            let extension = std::path::Path::new(path).extension()?.to_str()?;
            (!path.contains("..")
                && !path.starts_with('/')
                && extension.chars().any(char::is_alphabetic))
            .then(|| path.to_owned())
        })
        .collect()
}

pub(super) fn intent_only(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    let tail = lower
        .lines()
        .rev()
        .find(|s| !s.trim().is_empty())
        .unwrap_or("")
        .trim();
    [
        "i'll ",
        "i’ll ",
        "i will ",
        "let me ",
        "now i'll ",
        "now i’ll ",
        "creating ",
        "updating ",
        "running ",
    ]
    .iter()
    .any(|s| tail.starts_with(s))
        && !tail.contains('?')
        && !tail.contains("complete")
}

pub fn unverified_claim(task: &str, text: &str, history: &[Value]) -> Option<String> {
    let results = history
        .iter()
        .filter(|m| m["role"] == "tool")
        .filter_map(|m| {
            serde_json::from_str::<Value>(m["content"].as_str()?)
                .ok()
                .map(|v| (m["tool_name"].as_str().unwrap_or(""), v))
        })
        .collect::<Vec<_>>();
    let lower = text.to_lowercase();
    // Match public assertions at sentence starts. Quoted examples and general
    // explanations of exit codes are not claims that this task executed anything.
    let statements = lower
        .lines()
        .filter(|line| !line.trim_start().starts_with(['>', '`', '"']))
        .flat_map(|line| line.split(". "))
        .map(|line| line.trim_start_matches([' ', '*', '-']).trim())
        .collect::<Vec<_>>();
    let assertion = |patterns: &[&str]| {
        patterns
            .iter()
            .any(|p| statements.iter().any(|line| line.starts_with(p)))
    };
    let claims_edit = assertion(&[
        "i changed",
        "i updated",
        "i fixed",
        "i modified",
        "i created",
        "i applied",
        "i've changed",
        "i've updated",
        "i've fixed",
        "i’ve changed",
        "i’ve updated",
        "i’ve fixed",
        "changes applied",
        "file updated",
    ]);
    let claims_run = assertion(&["i ran", "i executed", "i've run", "i’ve run"]);
    if claims_edit
        && !results
            .iter()
            .any(|(n, v)| *n == "apply_patch" && v["ok"] == true)
    {
        return Some("Unverified action claim: no successful file edit is recorded. The model response is not execution evidence.".into());
    }
    if claims_run
        && !results.iter().any(|(n, v)| {
            matches!(*n, "run_command" | "exec_command" | "write_stdin")
                && v["exitCode"].is_number()
        })
    {
        return Some("Unverified action claim: no executed command is recorded. The model response is not execution evidence.".into());
    }
    let request = task.to_lowercase();
    if ["this project", "this repository", "this repo"]
        .iter()
        .any(|p| request.contains(p))
        && ["inspect", "architecture", "stack"]
            .iter()
            .any(|p| request.contains(p))
        && results.is_empty()
        && !lower.contains("workspace")
    {
        return Some("Project answer is unverified: no workspace files were inspected.".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn result(name: &str, id: &str, value: Value) -> Value {
        json!({"role":"tool","tool_name":name,"tool_call_id":id,"content":value.to_string()})
    }
    fn command(id: &str, verb: &str) -> Value {
        json!({"role":"assistant","tool_calls":[{"id":id,"function":{"name":"run_command","arguments":{"executable":"cargo","arguments":[verb]}}}]})
    }
    #[test]
    fn commands_require_actual_execution_and_are_separate_outcomes() {
        let requirements = Requirements::new("Run cargo test and cargo check");
        assert_eq!(requirements.missing(&[]).len(), 2);
        let history = [
            command("test", "test"),
            result("run_command", "test", json!({"ok":false,"exitCode":101})),
        ];
        assert_eq!(requirements.missing(&history).len(), 1);
        // Run-and-report accepts the recorded failure; cargo check is still missing.
        assert!(unverified_claim("Run tests", "I ran cargo test.", &history).is_none());
        assert!(unverified_claim("Run tests", "I ran cargo test.", &[]).is_some());
    }
    #[test]
    fn edits_need_verification_then_a_fresh_command() {
        let req = Requirements::new("Update README.md and run cargo test");
        let mut history = vec![
            command("test", "test"),
            result("run_command", "test", json!({"ok":true,"exitCode":0})),
            result(
                "apply_patch",
                "patch",
                json!({"ok":true,"verified":true,"path":"README.md"}),
            ),
        ];
        assert_eq!(req.missing(&history).len(), 1);
        history.extend([
            command("again", "test"),
            result("run_command", "again", json!({"ok":true,"exitCode":0})),
        ]);
        assert!(req.missing(&history).is_empty());
        assert!(
            !req.missing(&[result(
                "apply_patch",
                "patch",
                json!({"ok":true,"verified":false,"path":"README.md"})
            )])
            .is_empty()
        );
    }
    #[test]
    fn read_and_edit_clauses_do_not_require_edits_to_input_files() {
        let req = Requirements::new("Read Cargo.toml and update README.md");
        assert_eq!(req.paths, vec!["readme.md"]);
        assert!(
            req.missing(&[
                result(
                    "read_file",
                    "read",
                    json!({"ok":true,"path":"Cargo.toml","content":"actual"})
                ),
                result(
                    "apply_patch",
                    "patch",
                    json!({"ok":true,"verified":true,"path":"README.md"})
                )
            ])
            .is_empty()
        );
    }
    #[test]
    fn explicit_multiple_targets_and_input_reads_stay_separate() {
        let req = Requirements::new("Update README.md and src/lib.rs for v0.4");
        assert_eq!(req.paths, vec!["readme.md", "src/lib.rs"]);
        let req = Requirements::new("Read Cargo.toml and update README.md from src/lib.rs");
        assert_eq!(req.paths, vec!["readme.md"]);
        assert_eq!(req.read_paths, vec!["cargo.toml"]);
        assert!(
            !req.missing(&[
                result(
                    "read_file",
                    "read",
                    json!({"ok":true,"path":"other.rs","content":"unrelated"})
                ),
                result(
                    "apply_patch",
                    "patch",
                    json!({"ok":true,"verified":true,"path":"README.md"})
                )
            ])
            .is_empty()
        );
    }
    #[test]
    fn negative_constraints_never_require_an_edit_or_command() {
        let req =
            Requirements::new("Inspect this repository; do not modify README.md or run tests");
        assert!(req.inspect);
        assert!(!req.edit);
        assert!(req.commands.is_empty());
        let req = Requirements::new("Do not edit README.md. Run cargo test");
        assert!(!req.edit);
        assert_eq!(req.commands, vec!["test"]);
    }
    #[test]
    fn factual_answers_and_quoted_examples_are_not_execution_claims() {
        assert!(!Requirements::new("Hi").action());
        assert!(!Requirements::new("Write a short poem").action());
        assert!(!Requirements::new("Read me a short greeting").action());
        assert!(Requirements::new("Improve README.md").edit);
        assert!(Requirements::new("Fix it").edit);
        assert!(Requirements::new("Explain ownership and update README.md").edit);
        assert!(Requirements::new("Make README.md clearer").edit);
        assert!(!Requirements::new("Explain Rust ownership").action());
        assert!(!Requirements::new("What did you change and what was the test result? Answer from this conversation without using tools.").action());
        assert!(
            unverified_claim(
                "Explain cargo test",
                "Cargo test reports exit code 0 when all tests pass.",
                &[]
            )
            .is_none()
        );
        assert!(
            unverified_claim(
                "Explain strings",
                "> I ran the tests\nThis is an example string.",
                &[]
            )
            .is_none()
        );
    }
}

/// Compact deterministic result. Supporting records are actual task-local tool
/// observations; original observations remain in the persisted transcript.
#[derive(Debug, Serialize)]
pub struct Report {
    pub satisfied: bool,
    pub satisfied_requirements: Vec<String>,
    pub unsatisfied: Vec<String>,
    pub evidence: Vec<serde_json::Value>,
}
impl Requirements {
    pub fn report(&self, history: &[Value]) -> Report {
        let unsatisfied = self.missing(history);
        let evidence:Vec<_>=history.iter().filter(|m|m["role"]=="tool").filter_map(|m| {
            let value=serde_json::from_str::<Value>(m["content"].as_str()?).ok()?;
            Some(serde_json::json!({"tool":m["tool_name"],"callId":m["tool_call_id"],"ok":value["ok"],"exitCode":value["exitCode"],"path":value["path"],"verified":value["verified"]}))
        }).collect();
        let satisfied_requirements = self
            .missing(&[])
            .into_iter()
            .filter(|r| !unsatisfied.contains(r))
            .collect();
        Report {
            satisfied: unsatisfied.is_empty(),
            satisfied_requirements,
            unsatisfied,
            evidence,
        }
    }
}

#[cfg(test)]
mod read_identity_tests {
    use super::*;
    fn read(path: &str, ok: bool) -> Value {
        serde_json::json!({"role":"tool","tool_name":"read_file","content":serde_json::json!({"ok":ok,"path":path,"content":"real content"}).to_string()})
    }
    #[test]
    fn bare_filename_matches_one_successful_path_without_losing_explicit_scope() {
        let bare = Requirements::new("Read mod.rs");
        let actual = read("src/tools/agent/mod.rs", true);
        assert!(bare.missing(std::slice::from_ref(&actual)).is_empty());
        assert!(
            bare.missing(&[actual.clone(), actual.clone(), read("README.md", true)])
                .is_empty()
        );
        for instruction in ["Read src/tools/web/mod.rs", "Read mod.rs in src/tools/web/"] {
            assert!(
                !Requirements::new(instruction)
                    .missing(std::slice::from_ref(&actual))
                    .is_empty()
            );
        }
        assert!(
            !bare
                .missing(&[actual, read("src/tools/web/mod.rs", true)])
                .is_empty()
        );
        assert!(
            !bare
                .missing(&[read("src/tools/agent/mod.rs", false)])
                .is_empty()
        );
        assert!(
            !bare
                .missing(&[serde_json::json!({"role":"assistant","content":"I read mod.rs"})])
                .is_empty()
        );
    }
}

#[cfg(test)]
mod empty_file_tests {
    use super::*;
    #[test]
    fn verified_edit_alone_does_not_prove_an_explicit_empty_file_outcome() {
        let req = Requirements::new("Create an empty README.md; do not populate it");
        let result = |bytes| {
            serde_json::json!({"role":"tool","tool_name":"apply_patch","content":
            serde_json::json!({"ok":true,"verified":true,"path":"README.md","bytes":bytes}).to_string()})
        };
        assert!(!req.missing(&[result(8)]).is_empty());
        assert!(req.missing(&[result(0)]).is_empty());
        assert!(!req.missing(&[result(0), result(8)]).is_empty());
        for task in [
            "Create README.md with content; do not populate unrelated files",
            "Create README.md with content; do not create an empty file",
        ] {
            assert!(Requirements::new(task).missing(&[result(8)]).is_empty());
        }
        assert!(
            Requirements::new("Create README.md in an empty workspace")
                .missing(&[result(8)])
                .is_empty()
        );
    }
}
