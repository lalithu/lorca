//! Retain failures as observations, never prescribe a replacement action.
use crate::inference::Call;
use serde_json::{Value, json};
use std::collections::HashMap;
#[derive(Default)]
pub(super) struct Recovery {
    failed: HashMap<String, (Value, bool)>,
    consecutive: usize,
}
impl Recovery {
    fn key(call: &Call) -> String {
        format!("{}:{}", call.name, call.arguments)
    }
    pub fn stalled(&self) -> bool {
        self.consecutive >= 3
    }
    pub fn blocked(&mut self, call: &Call) -> Option<Value> {
        let (previous, denied) = self.failed.get(&Self::key(call))?;
        self.consecutive += 1;
        Some(
            json!({"ok":false,"executed":false,"blockedRepeat":true,"denied":denied,"previousResult":previous,
            "error":if *denied {"This action was denied. It was not run or submitted for approval again. Ask the user before requesting additional scope; do not bypass the denial."}else{"This identical action already failed and has not been rerun. Use the retained result to correct the call, choose another approach, or ask the user. A retry requires changed conditions."}}),
        )
    }
    pub fn observe(&mut self, call: &Call, result: &Value, denied: bool) {
        if result["ok"] == false {
            self.consecutive += 1;
            self.failed
                .insert(Self::key(call), (result.clone(), denied));
        } else {
            self.consecutive = 0;
        }
    }
    // Actual effects or an explicit user correction may change failure conditions.
    // Neither can silently lift a user denial within this run.
    pub fn changed(&mut self) {
        self.failed.retain(|_, (_, denied)| *denied);
        self.consecutive = 0;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_failures_reuse_evidence_and_denials_survive_changes() {
        let call = Call {
            id: "a".into(),
            name: "tool".into(),
            arguments: json!({"value":1}),
        };
        let mut r = Recovery::default();
        r.observe(
            &call,
            &json!({"ok":false,"output":"actual error","executed":true}),
            false,
        );
        let retained = r.blocked(&call).unwrap();
        assert_eq!(retained["executed"], false);
        assert_eq!(retained["previousResult"]["output"], "actual error");
        assert!(!r.stalled());
        r.blocked(&call);
        assert!(r.stalled());
        let corrected = Call {
            arguments: json!({"value":2}),
            ..call.clone()
        };
        assert!(r.blocked(&corrected).is_none());
        r.changed();
        assert!(r.blocked(&call).is_none());
        r.observe(&call, &json!({"ok":false}), true);
        r.changed();
        assert_eq!(r.blocked(&call).unwrap()["denied"], true);
    }
}
