use crate::tools::{arguments::parse, *};
pub struct RequestUserInput;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    question: String,
    #[serde(default)]
    recommendation: String,
    #[serde(default)]
    options: Vec<String>,
}

impl Tool for RequestUserInput {
    fn schema(&self) -> Value {
        schema(
            "request_user_input",
            "Pause this task with a concise question in the existing conversation. The user answers in their next message; no answer is assumed.",
            json!({"question":{"type":"string","maxLength":1000},"recommendation":{"type":"string","maxLength":500},"options":{"type":"array","maxItems":4,"items":{"type":"string","maxLength":200}}}),
            vec!["question"],
        )
    }
    fn prepare(&self, _: &Workspace, a: Value, l: &Limits) -> Result<Prepared> {
        self.prepare_optional(None, a, l)
    }
    fn prepare_optional(&self, _: Option<&Workspace>, a: Value, _: &Limits) -> Result<Prepared> {
        check_size(&a)?;
        let q: Args = parse(a.clone())?;
        if q.question.trim().is_empty()
            || q.question.len() > 1000
            || q.recommendation.len() > 500
            || q.options.len() > 4
            || q.options
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > 200)
        {
            return Err("Question must contain 1–1000 bytes".into());
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
                json!({"ok":true,"needsUserInput":true,"question":p.payload["question"],"recommendation":p.payload["recommendation"],"options":p.payload["options"]}),
            )
        })
    }
}

/// A folder choice grants a capability on the next task; it never runs a file action.
pub struct RequestWorkspace;
impl Tool for RequestWorkspace {
    fn schema(&self) -> Value {
        schema(
            "request_workspace",
            "Ask the user to choose a local folder when their task requires file or command tools. Pauses until they choose; do not invent a path.",
            json!({}),
            vec![],
        )
    }
    fn prepare(&self, _: &Workspace, a: Value, l: &Limits) -> Result<Prepared> {
        self.prepare_optional(None, a, l)
    }
    fn prepare_optional(&self, _: Option<&Workspace>, a: Value, _: &Limits) -> Result<Prepared> {
        if a.as_object().is_none_or(|o| !o.is_empty()) {
            return Err("request_workspace takes no arguments".into());
        }
        prepared(
            json!({"question":"Choose a folder so I can work with its files.","needsWorkspace":true}),
            false,
        )
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
                json!({"ok":true,"needsUserInput":true,"needsWorkspace":true,"question":p.payload["question"]}),
            )
        })
    }
}
