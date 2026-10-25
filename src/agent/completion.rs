//! Conservative completion guard; prose never supplies execution evidence.
use serde_json::Value;
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
            .any(|(n, v)| *n == "run_command" && v["ok"] == true)
    {
        return Some("Unverified action claim: no successful command is recorded. The model response is not execution evidence.".into());
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

/// Explicit requests to inspect the current workspace or run a command need
/// evidence from this interaction, rather than an older successful command.
pub fn unverified_request(task: &str, text: &str, current: &[Value]) -> Option<String> {
    let request = task.to_lowercase();
    let answer = text.to_lowercase();
    if [
        "workspace",
        "cannot",
        "can't",
        "unable",
        "not run",
        "not executed",
        "no changes needed",
    ]
    .iter()
    .any(|s| answer.contains(s))
        || answer.contains('?')
    {
        return None;
    }
    let results = current
        .iter()
        .filter(|m| m["role"] == "tool")
        .collect::<Vec<_>>();
    if request.starts_with("run ")
        && ["cargo", "test", "check", "clippy"]
            .iter()
            .any(|s| request.contains(s))
        && !results.iter().any(|m| m["tool_name"] == "run_command")
    {
        return Some("Task incomplete: this request has no verified command execution. Earlier test results do not verify a new run.".into());
    }
    if ["this project", "this repository", "this repo"]
        .iter()
        .any(|s| request.contains(s))
        && ["inspect", "architecture", "stack"]
            .iter()
            .any(|s| request.contains(s))
        && results.is_empty()
    {
        return Some("Project answer is unverified: the current workspace was not inspected for this request.".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn factual_answers_about_test_exit_codes_are_not_action_claims() {
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
        assert!(unverified_claim("Run tests", "I ran cargo test.", &[]).is_some());
    }
}
