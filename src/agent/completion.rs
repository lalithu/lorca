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

#[derive(Debug, Serialize)]
pub(super) struct Requirements {
    inspect: bool,
    directory_only: bool,
    edit: bool,
    paths: Vec<String>,
    read_paths: Vec<String>,
    commands: Vec<String>,
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
                    read_paths.extend(requested_paths(clause));
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
        Self {
            inspect: !explanatory
                && has(&["inspect", "review", "read", "examine"])
                && (workspace_action || !read_paths.is_empty() || lower.contains("directory")),
            directory_only: lower.contains("directory")
                && !lower.contains("repo")
                && !lower.contains("project"),
            edit,
            paths,
            read_paths,
            commands,
        }
    }
    pub fn action(&self) -> bool {
        self.inspect || self.edit || !self.commands.is_empty()
    }
    pub fn missing(&self, current: &[Value]) -> Vec<String> {
        let mut read = false;
        let mut read_paths = vec![];
        let mut patches = vec![];
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
                        patches.push(path.to_lowercase());
                        if !already_executed {
                            last_edit = Some(index);
                        }
                    }
                }
                Some("run_command") if result["exitCode"].is_number() => {
                    let id = &m["tool_call_id"];
                    for call in current
                        .iter()
                        .filter_map(|m| m["tool_calls"].as_array())
                        .flatten()
                        .filter(|c| &c["id"] == id)
                    {
                        commands.push((call["function"]["arguments"]["arguments"].clone(), index));
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
            if !read_paths.contains(path) {
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
        for name in &self.commands {
            if !commands.iter().any(|(args, index)| {
                if self.edit && last_edit.is_some_and(|edit| *index <= edit) {
                    return false;
                }
                name.is_empty()
                    || args
                        .as_array()
                        .is_some_and(|a| a.first().is_some_and(|v| v == name))
            }) {
                missing.push(format!("Execute the requested cargo {name} using run_command; report its actual exit status"));
            }
        }
        missing
    }
}

fn requested_paths(clause: &str) -> Vec<String> {
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
        && !results
            .iter()
            .any(|(n, v)| *n == "run_command" && v["exitCode"].is_number())
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
        // An exit 101 proves execution, never a successful command. Runtime's
        // unresolved error gate separately prevents Completed for this failure.
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
