//! Immutable task intent. No model request or model-authored scope changes.
use super::verification::Requirements;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Objective {
    pub original_instruction: String,
    pub requested_outcome: String,
    pub restrictions: Vec<String>,
    pub completion: Requirements,
}
impl Objective {
    pub fn new(instruction: &str) -> Self {
        let restrictions = instruction
            .lines()
            .flat_map(|s| s.split(';'))
            .filter(|s| {
                let s = s.to_lowercase();
                [
                    "do not ",
                    "don't ",
                    "dont ",
                    "never ",
                    "without ",
                    "read-only",
                ]
                .iter()
                .any(|m| s.contains(m))
            })
            .map(str::to_owned)
            .collect();
        Self {
            original_instruction: instruction.into(),
            requested_outcome: instruction.into(),
            restrictions,
            completion: Requirements::new(instruction),
        }
    }
}
