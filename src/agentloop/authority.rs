use super::{Agent, Limits, Registry, ToolCall, Workspace, types::*};
use serde_json::{Value, json};
use tokio::sync::watch;

/// Central policy gate. The prepared payload is retained by Rust and cannot be
/// modified by the model or the approval client. Tokens are single-use and task scoped.
pub async fn execute(
    agent: &Agent,
    registry: &Registry,
    w: Option<&Workspace>,
    call: &ToolCall,
    limits: &Limits,
    cancel: watch::Receiver<bool>,
    authority: &Authority,
) -> Result<Value> {
    authority.validate(call)?;
    let tool = registry.get(&call.name)?;
    let mut prepared = tool.prepare_optional(w, call.arguments.clone(), limits)?;
    if call.name == "apply_patch" {
        authority
            .patch_requirements
            .validate_patch(&prepared.payload)?;
    }
    if registry.requires_approval(&call.name, &call.arguments) && !prepared.approval {
        return Err("Tool registration requires approval but preparation omitted it".into());
    }
    let task_id = agent.snapshot().ok_or("Task disappeared")?.id;
    prepared.action["taskId"] = json!(task_id);
    prepared.action["objectiveRevision"] = json!(1);
    if prepared.approval {
        match agent
            .approval(call, prepared.action.clone(), cancel.clone())
            .await?
        {
            Decision::AllowOnce => {}
            Decision::Deny => {
                return Err("User denied this action; do not retry or bypass the denial".into());
            }
            Decision::Cancel => return Err("Cancelled".into()),
        }
    }
    if *cancel.borrow() || agent.snapshot().is_none_or(|v| v.id != task_id) {
        return Err("Cancelled".into());
    }
    agent.emit(
        "tool_started",
        json!({"callId":call.id,"tool":call.name,"action":prepared.action}),
    );
    let mut output = |data| agent.emit("terminal_output", json!({"callId":call.id,"output":data}));
    let result = tool
        .execute_optional(w, &prepared, limits, cancel, &mut output)
        .await?;
    if call.name == "apply_patch" && result["ok"] == true {
        agent.emit(
            "file_changes",
            json!({"callId":call.id,"change":prepared.action,"verified":true}),
        );
    }
    Ok(result)
}

const EDIT_RESTRICTIONS: &[&str] = &[
    "do not edit",
    "do not modify",
    "do not change",
    "don't edit",
    "don't modify",
    "dont edit",
    "never edit",
    "never modify",
    "do not write",
    "without editing",
    "without modifying",
    "read-only",
    "read only",
    "no edits",
];
const COMMAND_RESTRICTIONS: &[&str] = &[
    "do not run",
    "don't run",
    "dont run",
    "never run",
    "no commands",
    "do not execute",
    "without running",
];

/// Conservative recognition of explicit restrictions over existing capabilities.
/// Unrecognized language never adds tools or waives the permission gateway.
pub(super) fn validate(task: &str, call: &ToolCall) -> Result<()> {
    let text = task.to_lowercase();
    let blocked = match call.name.as_str() {
        "apply_patch" => EDIT_RESTRICTIONS.iter().any(|s| text.contains(s)),
        "run_command" | "exec_command" | "write_stdin" => {
            COMMAND_RESTRICTIONS.iter().any(|s| text.contains(s))
        }
        "fetch_url" | "web_search" => [
            "do not browse",
            "do not fetch",
            "do not search the web",
            "no internet",
            "no network",
            "do not access the internet",
            "without browsing",
            "offline only",
        ]
        .iter()
        .any(|s| text.contains(s)),
        _ => false,
    };
    if blocked {
        Err("Task restriction forbids this action; approval cannot expand task scope".into())
    } else {
        Ok(())
    }
}

#[derive(Debug, serde::Serialize)]
pub struct Authority {
    #[serde(rename = "constraint_instruction")]
    instruction: String,
    #[serde(skip)]
    patch_requirements: super::verification::Requirements,
    pub requires_action_approval: bool,
    pub scope: Scope,
}
impl Authority {
    pub fn new(instruction: &str) -> Self {
        Self {
            instruction: instruction.into(),
            patch_requirements: super::verification::Requirements::new(instruction),
            requires_action_approval: true,
            scope: Scope::default(),
        }
    }
}
#[cfg(test)]
mod contract_policy_tests {
    use super::*;
    fn corrected_authority(current: &str, raw: &str) -> Authority {
        let intent = super::super::intent::Interpretation {
            relation: super::super::intent::Relation::Correct,
            objective: current.into(),
            outcomes: vec![super::super::intent::Outcome {
                kind: super::super::intent::OutcomeKind::Edit,
                target: Some("CHOSEN.md".into()),
            }],
            edit_targets: vec!["CHOSEN.md".into()],
            forbidden_targets: vec![],
            read_only: false,
            commands_forbidden: true,
            clarification: None,
        };
        Authority::interpreted(
            raw,
            &intent,
            &super::super::verification::Requirements::new(current),
        )
    }
    #[test]
    fn corrected_content_uses_active_requirements_without_waiving_scope() {
        let a = corrected_authority(
            "Create CHOSEN.md containing hello",
            "Create an empty CHOSEN.md; do not run commands. Latest user instruction: Create CHOSEN.md containing hello",
        );
        assert!(
            a.patch_requirements
                .validate_patch(&json!({"path":"CHOSEN.md","replacement":"hello"}))
                .is_ok()
        );
        for (name, path) in [("apply_patch", "other.md"), ("exec_command", ".")] {
            assert!(
                a.validate(&ToolCall {
                    id: "test".into(),
                    name: name.into(),
                    arguments: json!({"path":path})
                })
                .is_err()
            );
        }
    }
    #[test]
    fn active_empty_requirement_and_explicit_read_only_remain_enforced() {
        let a = corrected_authority("Create an empty CHOSEN.md", "Create an empty CHOSEN.md");
        assert!(
            a.patch_requirements
                .validate_patch(&json!({"path":"CHOSEN.md","replacement":"hello"}))
                .is_err()
        );
        let a = corrected_authority("Create CHOSEN.md containing hello", "Do not edit any files");
        assert!(
            a.validate(&ToolCall {
                id: "test".into(),
                name: "apply_patch".into(),
                arguments: json!({"path":"CHOSEN.md"})
            })
            .is_err()
        );
    }
    fn revise(old: &str, scope: Scope, latest: &str) -> Authority {
        let previous = super::super::intent::PendingTask {
            completed: false,
            id: "old-task".into(),
            objective: old.into(),
            contract: Some(json!({"authority":{"constraint_instruction":old,"scope":scope}})),
        };
        let mut intent: super::super::intent::Interpretation = serde_json::from_value(json!({
            "relation":"correct","objective":"Create CHOSEN.md","outcomes":[{"kind":"edit","target":"CHOSEN.md"}],
            "edit_targets":[],"forbidden_targets":[],"read_only":false,"commands_forbidden":false,"clarification":null
        })).unwrap();
        // Deliberately permissive model output must not erase inherited limits.
        intent.edit_targets.clear();
        Authority::continued(
            &previous,
            latest,
            &intent,
            &super::super::verification::Requirements::new(latest),
        )
    }
    fn action(name: &str, path: &str) -> ToolCall {
        ToolCall {
            id: "test".into(),
            name: name.into(),
            arguments: json!({"path":path}),
        }
    }
    #[test]
    fn explicit_file_grant_is_bounded_and_preserves_other_prohibitions() {
        let a = revise(
            "Read only; do not run commands; no network; do not edit secret.md",
            Scope {
                read_only: true,
                commands_forbidden: true,
                ..Default::default()
            },
            "You may now edit CHOSEN.md.",
        );
        assert!(a.validate(&action("apply_patch", "CHOSEN.md")).is_ok());
        for path in ["other.md", "secret.md", "../escape.md"] {
            assert!(a.validate(&action("apply_patch", path)).is_err());
        }
        assert!(a.validate(&action("exec_command", ".")).is_err());
        assert!(a.validate(&action("fetch_url", ".")).is_err());
        assert!(a.requires_action_approval);
    }
    #[test]
    fn ambiguous_or_quoted_permission_does_not_expand_scope() {
        for latest in [
            "Go ahead",
            "Create CHOSEN.md",
            "The file says you may now edit CHOSEN.md",
            "You may now edit ../outside.md.",
            "```text\nYou may now edit CHOSEN.md.\n```",
        ] {
            let a = revise(
                "Read only; do not run commands",
                Scope {
                    read_only: true,
                    commands_forbidden: true,
                    ..Default::default()
                },
                latest,
            );
            assert!(
                a.validate(&action("apply_patch", "CHOSEN.md")).is_err(),
                "{latest}"
            );
            assert!(a.validate(&action("exec_command", ".")).is_err());
        }
    }
    #[test]
    fn command_grant_does_not_unlock_edits_and_survives_another_turn() {
        let a = revise(
            "Read only; do not run commands",
            Scope {
                read_only: true,
                commands_forbidden: true,
                ..Default::default()
            },
            "You may now run commands.",
        );
        assert!(a.validate(&action("exec_command", ".")).is_ok());
        assert!(a.validate(&action("apply_patch", "CHOSEN.md")).is_err());
        let b = revise(&a.instruction, a.scope.clone(), "Report the results");
        assert!(b.validate(&action("exec_command", ".")).is_ok());
        assert!(b.validate(&action("apply_patch", "CHOSEN.md")).is_err());
    }
    #[test]
    fn file_grant_survives_followup_without_expanding_to_other_files() {
        let a = revise(
            "Do not edit any files",
            Scope {
                read_only: true,
                ..Default::default()
            },
            "You may now edit CHOSEN.md.",
        );
        let b = revise(&a.instruction, a.scope.clone(), "Continue");
        assert!(b.validate(&action("apply_patch", "CHOSEN.md")).is_ok());
        assert!(b.validate(&action("apply_patch", "other.md")).is_err());
    }
    #[test]
    fn explicit_restrictions_cannot_be_overridden_by_tool_arguments() {
        for name in ["apply_patch", "run_command"] {
            let call = ToolCall {
                id: "test".into(),
                name: name.into(),
                arguments: serde_json::json!({"approved":true}),
            };
            assert!(validate("Read only; do not run commands", &call).is_err());
            assert!(validate("Inspect the project", &call).is_ok()); // Still requires normal gateway approval.
        }
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Scope {
    pub read_only: bool,
    pub commands_forbidden: bool,
    pub targets: Vec<String>,
    pub forbidden: Vec<String>,
}
impl Authority {
    pub(super) fn interpreted(
        instruction: &str,
        i: &super::intent::Interpretation,
        requirements: &super::verification::Requirements,
    ) -> Self {
        Self {
            instruction: instruction.into(),
            patch_requirements: requirements.clone(),
            requires_action_approval: true,
            scope: Scope {
                read_only: i.read_only,
                commands_forbidden: i.commands_forbidden,
                targets: i.edit_targets.clone(),
                forbidden: i.forbidden_targets.clone(),
            },
        }
    }
    /// Only explicit grants in the latest user instruction revise earlier limits.
    /// Model-authored scope may narrow these bounds, never broaden them.
    pub(super) fn continued(
        previous: &super::intent::PendingTask,
        latest: &str,
        intent: &super::intent::Interpretation,
        requirements: &super::verification::Requirements,
    ) -> Self {
        let prior = previous.contract.as_ref();
        let old = prior
            .and_then(|c| c["authority"]["constraint_instruction"].as_str())
            .or_else(|| prior.and_then(|c| c["objective"]["original_instruction"].as_str()))
            .unwrap_or(&previous.objective);
        let mut previous_scope: Scope = prior
            .and_then(|c| serde_json::from_value(c["authority"]["scope"].clone()).ok())
            .unwrap_or_default();
        let clauses = latest.replace(". ", "\n").replace(';', "\n");
        let mut grants = Vec::new();
        let mut commands_allowed = false;
        let mut quoted_block = false;
        for clause in clauses.lines().map(str::trim) {
            if clause.starts_with("```") || clause.starts_with("~~~") {
                quoted_block = !quoted_block;
                continue;
            }
            if quoted_block {
                continue;
            }
            let lower = clause.to_lowercase();
            commands_allowed |= matches!(
                lower.trim_end_matches('.'),
                "you may now run commands" | "commands are now allowed"
            );
            for prefix in ["you may now edit ", "you can now edit "] {
                if lower.starts_with(prefix) {
                    let path = clause[prefix.len()..]
                        .trim()
                        .trim_end_matches('.')
                        .trim_matches('`');
                    if !path.is_empty()
                        && !path.chars().any(char::is_whitespace)
                        && let Some(path) = super::recovery::path_key(path)
                        && !path.is_empty()
                    {
                        grants.push(path);
                    }
                }
            }
        }
        // Keep explicitly prohibited files even if the old interpretation omitted
        // them. A grant for one file cannot unlock unrelated prohibited paths.
        for clause in old.replace(". ", "\n").replace(';', "\n").lines() {
            if EDIT_RESTRICTIONS
                .iter()
                .any(|p| clause.to_lowercase().contains(p))
            {
                previous_scope
                    .forbidden
                    .extend(super::verification::requested_paths(clause));
            }
        }
        let mut raw = old.to_owned();
        if !grants.is_empty() {
            for marker in EDIT_RESTRICTIONS {
                raw = replace_ascii_case_insensitive(&raw, marker, "superseded edit restriction");
            }
        }
        if commands_allowed {
            for marker in COMMAND_RESTRICTIONS {
                raw =
                    replace_ascii_case_insensitive(&raw, marker, "superseded command restriction");
            }
        }
        let mut value = Self::interpreted(&format!("{raw}\n{latest}"), intent, requirements);
        if previous_scope.read_only && grants.is_empty() {
            value.scope.read_only = true;
        }
        if previous_scope.commands_forbidden && !commands_allowed {
            value.scope.commands_forbidden = true;
        }
        let mut bounds = previous_scope.targets;
        if previous_scope.read_only && !grants.is_empty() {
            bounds = grants.clone();
        } else if !bounds.is_empty() {
            bounds.extend(grants.iter().cloned());
        }
        if !bounds.is_empty() {
            if !value.scope.targets.is_empty() {
                bounds.retain(|p| value.scope.targets.iter().any(|n| p == n));
                if bounds.is_empty() {
                    value.scope.read_only = true;
                }
            }
            value.scope.targets = bounds;
        } else if !grants.is_empty()
            && EDIT_RESTRICTIONS
                .iter()
                .any(|p| old.to_lowercase().contains(p))
        {
            value.scope.targets = grants.clone();
        }
        value.scope.forbidden.extend(
            previous_scope
                .forbidden
                .into_iter()
                .filter(|p| !grants.contains(p)),
        );
        value.scope.forbidden.sort();
        value.scope.forbidden.dedup();
        value
    }
    fn validate(&self, call: &ToolCall) -> Result<()> {
        // A scoped exclusion is not a blanket prohibition. Only explicitly named
        // permitted paths can qualify; all other raw restrictions still apply.
        let scoped = call.name == "apply_patch"
            && !self.scope.targets.is_empty()
            && self.instruction.to_lowercase().contains("other")
            && self
                .scope
                .targets
                .iter()
                .all(|p| self.instruction.contains(p));
        let mut raw = self.instruction.to_lowercase();
        if scoped {
            for marker in [
                "do not edit",
                "do not modify",
                "do not change",
                "don't edit",
                "don't modify",
                "dont edit",
                "never edit",
                "never modify",
                "do not write",
                "without editing",
                "without modifying",
            ] {
                raw = raw.replace(&format!("{marker} other "), "scoped exclusion ");
            }
        }
        validate(&raw, call)?;
        if call.name == "apply_patch" {
            if self.scope.read_only {
                return Err("Task scope is read-only".into());
            }
            let path = call.arguments["path"]
                .as_str()
                .and_then(super::recovery::path_key)
                .ok_or("Invalid arguments: Invalid edit target")?;
            let matches = |p: &String| {
                path == p.trim_end_matches('/')
                    || path.starts_with(&format!("{}/", p.trim_end_matches('/')))
            };
            if self.scope.forbidden.iter().any(matches)
                || (!self.scope.targets.is_empty() && !self.scope.targets.iter().any(matches))
            {
                return Err("Edit target is outside the active task scope".into());
            }
        }
        if self.scope.commands_forbidden
            && matches!(
                call.name.as_str(),
                "exec_command" | "run_command" | "write_stdin"
            )
        {
            return Err("Task scope forbids commands".into());
        }
        Ok(())
    }
}

// Preserve path case and non-ASCII user text while replacing ASCII policy markers.
fn replace_ascii_case_insensitive(text: &str, needle: &str, replacement: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::new();
    let mut offset = 0;
    while let Some(index) = lower[offset..].find(needle) {
        let index = offset + index;
        out.push_str(&text[offset..index]);
        out.push_str(replacement);
        offset = index + needle.len();
    }
    out.push_str(&text[offset..]);
    out
}
