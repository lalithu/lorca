use crate::tools::{arguments::parse, *};
use serde::Serialize;
pub struct UpdatePlan;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    steps: Vec<Step>,
    #[serde(default)]
    checkpoint: Option<WorkingCheckpoint>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    step: String,
    status: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorkingCheckpoint {
    approach: String,
    decisions: Vec<String>,
    uncertainties: Vec<String>,
    next_action: String,
}
impl WorkingCheckpoint {
    pub fn valid(&self) -> bool {
        !self.approach.trim().is_empty()
            && self.approach.len() <= 600
            && !self.next_action.trim().is_empty()
            && self.next_action.len() <= 300
            && [&self.decisions, &self.uncertainties].iter().all(|items| {
                items.len() <= 6 && items.iter().all(|s| !s.trim().is_empty() && s.len() <= 300)
            })
    }
    pub fn schema() -> Value {
        json!({"type":"object","additionalProperties":false,"required":["approach","decisions","uncertainties","nextAction"],
            "properties":{"approach":{"type":"string","maxLength":600},"nextAction":{"type":"string","maxLength":300},
            "decisions":{"type":"array","maxItems":6,"items":{"type":"string","maxLength":300}},
            "uncertainties":{"type":"array","maxItems":6,"items":{"type":"string","maxLength":300}}}})
    }
}
impl Tool for UpdatePlan {
    fn schema(&self) -> Value {
        schema(
            "update_plan",
            "Record or replace a short task plan and optional working checkpoint (approach, decisions, uncertainties, nextAction). Use only for useful task progress, not repeated reconsideration. These are untrusted notes, never execution evidence or permission.",
            json!({"checkpoint":WorkingCheckpoint::schema(),"steps":{"type":"array","minItems":1,"maxItems":8,"items":{"type":"object","properties":{"step":{"type":"string"},"status":{"type":"string","enum":["pending","in_progress","completed"]}},"required":["step","status"],"additionalProperties":false}}}),
            vec!["steps"],
        )
    }
    fn prepare(&self, _: &Workspace, a: Value, l: &Limits) -> Result<Prepared> {
        self.prepare_optional(None, a, l)
    }
    fn prepare_optional(&self, _: Option<&Workspace>, a: Value, _: &Limits) -> Result<Prepared> {
        check_size(&a)?;
        let plan: Args = parse(a.clone())?;
        if plan.checkpoint.as_ref().is_some_and(|c| !c.valid())
            || plan.steps.is_empty()
            || plan.steps.len() > 8
            || plan.steps.iter().any(|s| {
                s.step.trim().is_empty()
                    || s.step.len() > 300
                    || !["pending", "in_progress", "completed"].contains(&s.status.as_str())
            })
            || plan
                .steps
                .iter()
                .filter(|s| s.status == "in_progress")
                .count()
                > 1
        {
            return Err("Use 1–8 short steps with valid status and at most one in_progress".into());
        }
        prepared(a, false)
    }
    fn execute<'a>(
        &'a self,
        _: &'a Workspace,
        p: &'a Prepared,
        l: &'a Limits,
        c: watch::Receiver<bool>,
        o: &'a mut (dyn FnMut(Value) + Send),
    ) -> FutureResult<'a, Value> {
        self.execute_optional(None, p, l, c, o)
    }
    fn execute_optional<'a>(
        &'a self,
        _: Option<&'a Workspace>,
        p: &'a Prepared,
        _: &'a Limits,
        c: watch::Receiver<bool>,
        _: &'a mut (dyn FnMut(Value) + Send),
    ) -> FutureResult<'a, Value> {
        Box::pin(async move {
            if *c.borrow() {
                return Err("Cancelled".into());
            }
            Ok(
                json!({"ok":true,"plan":p.payload["steps"],"checkpoint":p.payload["checkpoint"],"verifiedCompletion":false}),
            )
        })
    }
}
