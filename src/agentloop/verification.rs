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
    // Only references extracted from user instructions; model labels never enter here.
    read_paths: Vec<String>,
    read_range: Option<(u64, u64)>,
    unresolved_replacement: bool,
    read_collections: Vec<super::inspection::Collection>,
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
        // Preserve exact spelling, quoted names and extensionless references.
        let mut read_paths = super::inspection::explicit_files(task);
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
                && (has(&["inspect", "review", "read", "examine"])
                    || positive.contains("look at "))
                && (workspace_action || !read_paths.is_empty() || lower.contains("directory")),
            directory_only: lower.contains("directory")
                && !lower.contains("repo")
                && !lower.contains("project"),
            edit,
            paths,
            empty_paths,
            read_range: (read_paths.len() == 1)
                .then(|| requested_range(task))
                .flatten(),
            read_paths,
            unresolved_replacement: false,
            read_collections: super::inspection::collections(task),
            report_only: !edit_requested,
            commands,
        }
    }
    pub(super) fn interpreted(task: &str, intent: &super::intent::Interpretation) -> Self {
        use super::intent::OutcomeKind::*;
        let mut value = Self::new(task);
        value.inspect |= intent.outcomes.iter().any(|o| o.kind == Inspect);

        if !value.report_only {
            value.edit |= intent
                .outcomes
                .iter()
                .any(|o| matches!(o.kind, Edit | RepairVerify));
        }
        // Model outcomes describe the work. Concrete requirements are grounded
        // exclusively in the user's instruction.
        value.report_only = !value.edit;
        value
    }
    pub(super) fn revise_inspection(&mut self, instructions: &[String]) {
        self.read_paths.clear();
        self.read_range = None;
        self.unresolved_replacement = false;
        self.read_collections.clear();
        for instruction in instructions {
            let mut next = Self::new(instruction);
            let replacement = instruction
                .to_ascii_lowercase()
                .split(|c: char| !c.is_alphabetic())
                .any(|word| word == "instead");
            if replacement && !next.read_paths.is_empty() {
                let lower = instruction.to_ascii_lowercase();
                if let Some(index) = lower.find("instead of ") {
                    let old = super::inspection::explicit_files(&format!(
                        "Read {}",
                        &instruction[index + "instead of ".len()..]
                    ));
                    let before = self.read_paths.len();
                    self.read_paths.retain(|path| !old.contains(path));
                    self.unresolved_replacement = before == self.read_paths.len() && before > 0;
                } else if self.read_paths.len() <= 1 {
                    self.read_paths.clear();
                    self.unresolved_replacement = false;
                } else {
                    self.unresolved_replacement = true;
                }
            }
            if replacement && !next.read_collections.is_empty() {
                self.read_collections.clear();
            }
            self.inspect |= next.inspect;
            if !next.read_paths.is_empty() {
                self.read_range = if self.read_paths.is_empty() {
                    next.read_range
                } else {
                    None
                };
                self.read_paths.append(&mut next.read_paths);
            }
            for collection in next.read_collections {
                if !self.read_collections.contains(&collection) {
                    self.read_collections.push(collection);
                }
            }
        }
        self.read_paths.sort();
        self.read_paths.dedup();
    }

    pub(super) fn terminal_read_blocker(&self, current: &[Value]) -> Option<String> {
        let complete = super::inspection::covered_reads(current, self.read_range);
        let complete_files = super::inspection::complete_reads(current);
        super::inspection::observations(current, "read_file")
            .iter()
            .find_map(|result| {
                let path = result["path"].as_str()?;
                let error = result["error"].as_str()?;
                (result["ok"] != true
                    && (error == "File is not UTF-8 text" || error == "Expected a regular file")
                    && (self.read_paths.iter().any(|requested| {
                        !read_satisfies(requested, &complete)
                            && read_satisfies(requested, &[path.into()])
                    }) || self.read_collections.iter().any(|collection| {
                        collection.coverage != super::intent::InspectionCoverage::Relevant
                            && !complete_files.iter().any(|actual| actual == path)
                            && collection.lists_member(current, path)
                    })))
                .then(|| format!("Could not read {path}: {error}"))
            })
    }

    pub(crate) fn requires_edit(&self) -> bool {
        self.edit
    }
    pub(super) fn requests_command(&self) -> bool {
        !self.commands.is_empty()
    }
    pub(super) fn reports_failure(&self) -> bool {
        self.report_only && !self.commands.is_empty()
    }
    pub fn action(&self) -> bool {
        self.inspect
            || self.edit
            || !self.commands.is_empty()
            || !self.read_paths.is_empty()
            || !self.read_collections.is_empty()
    }
    // "Relevant files" is open-ended. Only explicit coverage can close the
    // tool phase before the model offers a final answer.
    pub(super) fn bounded_inspection_complete(&self, current: &[Value]) -> bool {
        if self.edit
            || !self.commands.is_empty()
            || (self.read_paths.is_empty() && self.read_collections.is_empty())
            || self.read_collections.iter().any(|collection| {
                collection.coverage == super::intent::InspectionCoverage::Relevant
            })
        {
            return false;
        }
        let complete = super::inspection::covered_reads(current, self.read_range);
        self.read_paths
            .iter()
            .all(|path| read_satisfies(path, &complete))
    }
    pub(super) fn validate_patch(&self, payload: &Value) -> Result<(), String> {
        let path = normalized_path(payload["path"].as_str().unwrap_or(""));
        if self.empty_paths.contains(&path) && payload["replacement"] != "" {
            return Err("Task constraint: the requested file must be empty. Use new_text as an empty string; nothing was executed.".into());
        }
        Ok(())
    }
    pub fn missing(&self, current: &[Value]) -> Vec<String> {
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
        let mut missing: Vec<_> = self
            .inspection_status(current)
            .into_iter()
            .flat_map(|requirement| requirement.missing)
            .collect();
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
        let successful = super::inspection::covered_reads(current, self.read_range);
        for requested in &self.read_paths {
            if read_satisfies(requested, &successful) {
                continue;
            }
            if let Some(error) = results
                .iter()
                .rev()
                .filter(|v| v["ok"] != true)
                .find_map(|v| {
                    let path = v["path"].as_str()?.to_owned();
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

fn requested_range(task: &str) -> Option<(u64, u64)> {
    let words: Vec<_> = task.split_whitespace().collect();
    for words in words.windows(2) {
        if words[0].eq_ignore_ascii_case("bytes") {
            let Some((start, end)) = words[1].trim_end_matches([',', '.', ';']).split_once("..")
            else {
                continue;
            };
            let (start, end) = (start.parse::<u64>().ok()?, end.parse::<u64>().ok()?);
            return (start < end).then_some((start, end));
        }
    }
    words.windows(3).find_map(|w| {
        (w[0].eq_ignore_ascii_case("first") && w[2].eq_ignore_ascii_case("bytes"))
            .then(|| w[1].parse::<u64>().ok().filter(|n| *n > 0).map(|n| (0, n)))
            .flatten()
    })
}

fn normalized_path(path: &str) -> String {
    super::recovery::path_key(path)
        .unwrap_or_else(|| path.to_owned())
        .to_lowercase()
}

fn read_satisfies(requested: &str, observed: &[String]) -> bool {
    let qualified = requested.contains('/');
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
    !qualified
        && observed
            .iter()
            .filter(|p| p.rsplit('/').next() == Some(requested.as_str()))
            .count()
            == 1
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
                    json!({"ok":true,"path":"Cargo.toml","content":"actual","offset":0,"totalBytes":6,"truncated":false})
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
        assert_eq!(req.read_paths, vec!["Cargo.toml"]);
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
    pub inspection_requirements: Vec<InspectionCheck>,
    pub unsatisfied: Vec<String>,
    pub evidence: Vec<serde_json::Value>,
}
#[derive(Debug, Serialize)]
pub struct InspectionCheck {
    id: String,
    #[serde(flatten)]
    target: InspectionRequirement,
    satisfied: bool,
    missing: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum InspectionRequirement {
    ExplicitFile {
        path: String,
        required_range: Option<(u64, u64)>,
    },
    Collection {
        scope: super::inspection::Collection,
    },
    Unresolved {
        description: String,
    },
    Subject {
        directory: Option<String>,
    },
}

impl InspectionCheck {
    fn new(id: String, target: InspectionRequirement, missing: Vec<String>) -> Self {
        Self {
            id,
            target,
            satisfied: missing.is_empty(),
            missing,
        }
    }
}

impl Requirements {
    pub(super) fn inspection_contract(&self, instructions: &[String]) -> Vec<Value> {
        let declared: Vec<_> = instructions
            .iter()
            .map(|instruction| Self::new(instruction).inspection_status(&[]))
            .collect();
        self.inspection_status(&[]).into_iter().map(|requirement| {
            let origins = declared.iter().enumerate()
                .filter(|(_, candidates)| candidates.iter().any(|candidate| candidate.id == requirement.id))
                .map(|(i, _)| serde_json::json!({"instruction":instructions[i],"instructionIndex":i,"source":"user"}))
                .collect::<Vec<_>>();
            let mut value = serde_json::json!(requirement);
            let record = value.as_object_mut().unwrap();
            record.remove("missing");
            record.remove("satisfied");
            record.insert("origins".into(), serde_json::json!(origins));
            value
        }).collect()
    }

    fn inspection_status(&self, history: &[Value]) -> Vec<InspectionCheck> {
        let complete = super::inspection::covered_reads(history, self.read_range);
        let mut requirements = vec![];
        for path in &self.read_paths {
            let missing = if read_satisfies(path, &complete) {
                vec![]
            } else {
                vec![format!(
                    "Read the requested real file {path} with read_file and cover the required bytes"
                )]
            };
            requirements.push(InspectionCheck::new(
                format!("file:{path}"),
                InspectionRequirement::ExplicitFile {
                    path: path.clone(),
                    required_range: self.read_range,
                },
                missing,
            ));
        }
        for collection in &self.read_collections {
            requirements.push(InspectionCheck::new(
                format!(
                    "collection:{:?}:{}",
                    collection.coverage, collection.directory
                ),
                if collection.unresolved {
                    InspectionRequirement::Unresolved {
                        description: "collection directory".into(),
                    }
                } else {
                    InspectionRequirement::Collection {
                        scope: collection.clone(),
                    }
                },
                collection.missing(history),
            ));
        }
        if self.unresolved_replacement {
            requirements.push(InspectionCheck::new(
                "inspection:unresolved-replacement".into(),
                InspectionRequirement::Unresolved {
                    description: "which prior target the correction replaces".into(),
                },
                vec!["Specify which prior file the correction replaces".into()],
            ));
        }
        if requirements.is_empty() && self.inspect {
            let missing = if self.directory_only {
                let listed = super::inspection::observations(history, "list_directory")
                    .iter()
                    .any(|v| {
                        v["ok"] == true
                            && v["error"].is_null()
                            && v["truncated"] == false
                            && v["entries"].is_array()
                            && v["path"]
                                .as_str()
                                .and_then(super::recovery::path_key)
                                .as_deref()
                                == Some("")
                    });
                if listed {
                    vec![]
                } else {
                    vec!["List the workspace directory completely with list_directory".into()]
                }
            } else {
                let read = super::inspection::observations(history, "read_file")
                    .iter()
                    .any(|v| v["ok"] == true && v["error"].is_null() && v["content"].is_string());
                if read {
                    vec![]
                } else {
                    vec!["Inspect relevant real files with read_file; a listing or a promise is not file inspection".into()]
                }
            };
            requirements.push(InspectionCheck::new(
                "inspection:subject".into(),
                InspectionRequirement::Subject {
                    directory: self.directory_only.then(|| ".".into()),
                },
                missing,
            ));
        }
        requirements
    }

    pub fn report(&self, history: &[Value]) -> Report {
        let unsatisfied = self.missing(history);
        let inspection_requirements = self.inspection_status(history);
        let evidence:Vec<_>=history.iter().filter(|m|m["role"]=="tool").filter_map(|m| {
            let value=serde_json::from_str::<Value>(m["content"].as_str()?).ok()?;
            Some(serde_json::json!({"tool":m["tool_name"],"callId":m["tool_call_id"],"ok":value["ok"],
                "exitCode":value["exitCode"],"path":value["path"],"verified":value["verified"],
                "offset":value["offset"],"nextOffset":value["nextOffset"],"totalBytes":value["totalBytes"],
                "version":value["version"],"truncated":value["truncated"]}))
        }).collect();
        let satisfied_requirements = inspection_requirements
            .iter()
            .filter(|r| r.satisfied)
            .map(|r| r.id.clone())
            .collect();
        Report {
            satisfied: unsatisfied.is_empty(),
            satisfied_requirements,
            inspection_requirements,
            unsatisfied,
            evidence,
        }
    }
}

#[cfg(test)]
mod read_identity_tests {
    use super::*;
    fn read(path: &str, ok: bool) -> Value {
        serde_json::json!({"role":"tool","tool_name":"read_file","content":serde_json::json!({"ok":ok,"path":path,"content":"real content","offset":0,"totalBytes":12,"truncated":false}).to_string()})
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
#[cfg(test)]
mod inspection_target_tests {
    use super::*;
    use serde_json::json;

    fn interpreted(task: &str, target: Value) -> Requirements {
        let intent = serde_json::from_value(json!({
            "relation":"new", "objective":"Read hallucinated.md and inspect files present",
            "outcomes":[{"kind":"inspect","target":"files present","inspection":target}],
            "edit_targets":[],"forbidden_targets":[],"read_only":true,
            "commands_forbidden":true,"clarification":null
        }))
        .unwrap();
        Requirements::interpreted(task, &intent)
    }
    fn observation(tool: &str, value: Value) -> Value {
        json!({"role":"tool","tool_name":tool,"content":value.to_string()})
    }
    fn listing(directory: &str, paths: &[&str], truncated: bool) -> Value {
        observation(
            "list_directory",
            json!({"ok":true,"path":directory,"truncated":truncated,
            "entries":paths.iter().map(|p|json!({"path":p,"kind":"file"})).collect::<Vec<_>>() }),
        )
    }
    fn read(path: &str, offset: usize, total: usize, content: &str) -> Value {
        observation(
            "read_file",
            json!({"ok":true,"path":path,"offset":offset,
            "totalBytes":total,"version":"fixture","content":content,"truncated":offset+content.len()<total}),
        )
    }
    #[test]
    fn model_descriptions_and_type_labels_cannot_manufacture_file_requirements() {
        for description in [
            "files present",
            "source files",
            "repository contents",
            "phantom.md",
        ] {
            let intent = serde_json::from_value(json!({
                "relation":"new","objective":"Read hallucinated.md",
                "outcomes":[{"kind":"inspect","target":description,
                    "inspection":{"type":"file","path":description}}],
                "edit_targets":[],"forbidden_targets":[],"read_only":true,
                "commands_forbidden":true,"clarification":null
            }))
            .unwrap();
            let req = Requirements::interpreted("Inspect this repository", &intent);
            assert!(req.read_paths.is_empty(), "{description}");
            assert!(req.missing(&[read("README.md", 0, 4, "real")]).is_empty());
        }
    }
    #[test]
    fn explicit_references_survive_subject_labels_and_preserve_spelling() {
        for (task, path) in [
            ("Read README.md", "README.md"),
            ("Read `Project Notes.md`", "Project Notes.md"),
            ("Read LICENSE", "LICENSE"),
            ("Read the file named config", "config"),
            ("Read `設定.txt`", "設定.txt"),
            ("Read ./src/Exact.rs", "src/Exact.rs"),
            ("Read mod.rs in src/agent/", "src/agent/mod.rs"),
        ] {
            let req = interpreted(task, json!({"type":"subject"}));
            assert_eq!(req.read_paths, [path], "{task}");
            assert!(!req.missing(&[]).is_empty());
            assert!(
                req.missing(&[read(path, 0, 4, "real")]).is_empty(),
                "{task}"
            );
        }
        let req = interpreted("Read MISSING.md", Value::Null);
        assert!(
            req.missing(&[read("missing.md", 0, 4, "real")])
                .iter()
                .any(|m| m.contains("MISSING.md"))
        );
        assert!(
            !req.missing(&[observation(
                "read_file",
                json!({"ok":false,"path":"MISSING.md","error":"not found"})
            )])
            .is_empty()
        );
        assert!(
            req.read_blocker(&[observation(
                "read_file",
                json!({"ok":false,"path":"MISSING.md","error":"not found"})
            )])
            .is_some()
        );
    }
    #[test]
    fn full_collection_needs_a_complete_listing_and_all_complete_reads() {
        let req = interpreted("Read the files and report back", Value::Null);
        assert!(req.read_paths.is_empty());
        assert!(!req.missing(&[]).is_empty());
        let mut evidence = vec![listing(".", &["a.txt", "b.txt", "c.txt"], false)];
        assert!(!req.missing(&evidence).is_empty());
        evidence.push(read("a.txt", 0, 4, "real"));
        assert!(!req.missing(&evidence).is_empty());
        // A last excerpt does not cover byte zero; duplicate ranges don't close gaps.
        evidence.extend([
            read("b.txt", 2, 4, "al"),
            read("b.txt", 2, 4, "al"),
            read("c.txt", 0, 4, "re"),
        ]);
        assert_eq!(req.missing(&evidence).len(), 2);
        evidence.extend([read("b.txt", 0, 4, "re"), read("c.txt", 2, 4, "al")]);
        assert!(req.missing(&evidence).is_empty());
        evidence[0] = listing(".", &["a.txt", "b.txt", "c.txt"], true);
        assert!(!req.missing(&evidence).is_empty());
    }
    #[test]
    fn collection_scope_is_exact_and_empty_collections_are_valid() {
        let req = interpreted("Read all files in src/", Value::Null);
        let mut evidence = vec![
            listing("src", &["src/A.txt"], false),
            read("other/A.txt", 0, 4, "real"),
        ];
        assert!(!req.missing(&evidence).is_empty());
        evidence.push(read("src/A.txt", 0, 4, "real"));
        assert!(req.missing(&evidence).is_empty());
        let req = interpreted("Read the files", Value::Null);
        assert!(req.missing(&[listing(".", &[], false)]).is_empty());
        assert!(!req.missing(&[listing(".", &[], true)]).is_empty());
        assert!(!req.missing(&[listing("elsewhere", &[], false)]).is_empty());
    }
    #[test]
    fn relevant_inspection_does_not_require_every_repository_file() {
        let req = interpreted("Read relevant source files", Value::Null);
        assert!(
            req.missing(&[
                listing(".", &["a.rs", "b.rs"], false),
                read("a.rs", 0, 4, "real")
            ])
            .is_empty()
        );
    }
    #[test]
    fn unreadable_collection_members_and_changed_lengths_do_not_certify_coverage() {
        let req = interpreted("Read the files", Value::Null);
        let symlink = observation(
            "list_directory",
            json!({"ok":true,"path":".","truncated":false,
            "entries":[{"kind":"symlink","path":null,"name":"linked"}]}),
        );
        assert!(!req.missing(&[symlink]).is_empty());
        assert!(
            !req.missing(&[
                listing(".", &["a.txt"], false),
                read("a.txt", 0, 4, "real"),
                read("a.txt", 4, 8, "more")
            ])
            .is_empty()
        );
    }
    #[test]
    fn continuation_rebuilds_inspection_from_user_references_without_legacy_labels() {
        let mut req = interpreted("Continue and report back. Do not edit files.", Value::Null);
        req.revise_inspection(&[String::from("Read README.md. Do not edit files.")]);
        assert_eq!(req.read_paths, ["README.md"]);
        assert!(!req.requires_edit());
        assert!(req.missing(&[read("README.md", 0, 4, "real")]).is_empty());
        let mut req = interpreted("Continue", Value::Null);
        req.revise_inspection(&[String::from(
            "Inspect this repository, read the files, report back.",
        )]);
        assert!(req.read_paths.is_empty());
        assert!(!req.read_collections.is_empty());
        assert!(!req.missing(&[]).is_empty());
    }
}

#[cfg(test)]
mod repair_regressions {
    use super::*;
    use serde_json::json;

    #[test]
    fn coordinated_targets_and_revisions_preserve_exact_identity() {
        for request in [
            "Read README.md, index.html, and lalith.txt, then explain what each contains.",
            "Read README.md, index.html and lalith.txt",
            "Read README.md; then index.html and lalith.txt",
        ] {
            assert_eq!(
                Requirements::new(request).read_paths,
                ["README.md", "index.html", "lalith.txt"]
            );
        }
        assert_eq!(Requirements::new("Read README").read_paths, ["README"]);
        assert_eq!(Requirements::new("Read config").read_paths, ["config"]);
        let mut req = Requirements::new("Continue");
        req.revise_inspection(
            &[
                "Read OLD.md",
                "Read NEW.md instead.",
                "Keep going",
                "Also inspect config.toml",
                "Continue",
            ]
            .map(str::to_owned),
        );
        assert_eq!(req.read_paths, ["NEW.md", "config.toml"]);
    }

    fn observation(tool: &str, result: Value) -> Value {
        json!({"role":"tool","tool_name":tool,"content":result.to_string()})
    }

    fn range(path: &str, offset: usize, bytes: &str, total: usize, version: &str) -> Value {
        observation(
            "read_file",
            json!({"path":path,"ok":true,"offset":offset,"content":bytes,
            "totalBytes":total,"version":version,"truncated":offset+bytes.len()<total}),
        )
    }

    #[test]
    fn full_read_rejects_wrong_paths_gaps_and_mixed_same_size_versions() {
        let req = Requirements::new("Read ./a.txt");
        assert!(
            !req.missing(&[range("different/a.txt", 0, "abcdef", 6, "v")])
                .is_empty()
        );
        assert!(
            !req.missing(&[
                range("a.txt", 0, "abc", 6, "v"),
                range("a.txt", 3, "XYZ", 6, "v2")
            ])
            .is_empty()
        );
        assert!(
            !req.missing(&[
                range("a.txt", 0, "ab", 6, "v"),
                range("a.txt", 3, "def", 6, "v")
            ])
            .is_empty()
        );
        assert!(
            req.missing(&[
                range("a.txt", 2, "cdef", 6, "v"),
                range("a.txt", 0, "abc", 6, "v")
            ])
            .is_empty()
        );
        assert!(req.missing(&[range("a.txt", 0, "", 0, "empty")]).is_empty());
    }

    #[test]
    fn recursive_collection_requires_every_enumerated_directory_and_file() {
        let req = Requirements::new("Read all files recursively in `version.1`");
        let list = |path: &str, entries: Value| {
            observation(
                "list_directory",
                json!({
            "path":path,"ok":true,"truncated":false,"entries":entries}),
            )
        };
        let root = list(
            "version.1",
            json!([{"kind":"directory","path":"version.1/nested"}]),
        );
        let nested = list(
            "version.1/nested",
            json!([{"kind":"file","path":"version.1/nested/a.txt"}]),
        );
        assert!(!req.missing(std::slice::from_ref(&root)).is_empty());
        assert!(!req.missing(&[root.clone(), nested.clone()]).is_empty());
        assert!(
            req.missing(&[
                root,
                nested,
                range("version.1/nested/a.txt", 0, "real", 4, "v")
            ])
            .is_empty()
        );
    }
}
