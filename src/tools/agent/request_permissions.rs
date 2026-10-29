use crate::tools::{arguments::parse, *};
pub struct RequestPermissions;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    reason: String,
    scope: String,
}

impl Tool for RequestPermissions {
    fn schema(&self) -> Value {
        schema(
            "request_permissions",
            "Ask the user about a required scope change, then pause. Does NOT grant access or bypass the existing workspace, command allowlist, or approval gateway.",
            json!({"reason":{"type":"string"},"scope":{"type":"string"}}),
            vec!["reason", "scope"],
        )
    }
    fn prepare(&self, _: &Workspace, a: Value, l: &Limits) -> Result<Prepared> {
        self.prepare_optional(None, a, l)
    }
    fn prepare_optional(&self, _: Option<&Workspace>, a: Value, _: &Limits) -> Result<Prepared> {
        check_size(&a)?;
        let q: Args = parse(a.clone())?;
        if q.reason.trim().is_empty()
            || q.reason.len() > 1000
            || q.scope.trim().is_empty()
            || q.scope.len() > 500
        {
            return Err("Supply a short reason and requested scope".into());
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
                json!({"ok":true,"needsUserInput":true,"granted":false,"question":format!("Permission needed: {}. Requested scope: {}. Current permissions remain unchanged; select the required workspace or adjust supported permissions before continuing.",p.payload["reason"].as_str().unwrap_or(""),p.payload["scope"].as_str().unwrap_or(""))}),
            )
        })
    }
}
