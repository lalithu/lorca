//! Model-requested proactive checkpoint. Rust still owns context safety.
use crate::tools::*;
pub struct CompactContext;
impl Tool for CompactContext {
    fn schema(&self) -> Value {
        schema(
            "compact_context",
            "Request a fresh context checkpoint after important progress. The full original history remains searchable. Optionally use update_plan first to save concise working notes. This tool cannot delete history or grant permissions.",
            json!({}),
            vec![],
        )
    }
    fn prepare(&self, _: &Workspace, args: Value, l: &Limits) -> Result<Prepared> {
        self.prepare_optional(None, args, l)
    }
    fn prepare_optional(&self, _: Option<&Workspace>, args: Value, _: &Limits) -> Result<Prepared> {
        if args.as_object().is_none_or(|m| !m.is_empty()) {
            return Err("compact_context takes no arguments".into());
        }
        prepared(args, false)
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
        _: &'a Prepared,
        _: &'a Limits,
        c: watch::Receiver<bool>,
        _: &'a mut (dyn FnMut(Value) + Send),
    ) -> FutureResult<'a, Value> {
        Box::pin(async move {
            if *c.borrow() {
                return Err("Cancelled".into());
            }
            Ok(json!({"ok":true,"compactNow":true,
                "note":"Next inference will start with a fresh bounded checkpoint; original records remain in the archive."}))
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn request_is_read_only_and_does_not_authorize_any_action() {
        let tool = CompactContext;
        let l = Limits::default();
        let p = tool.prepare_optional(None, json!({}), &l).unwrap();
        assert!(!p.approval);
        let (_, cancelled) = watch::channel(false);
        let result = tool
            .execute_optional(None, &p, &l, cancelled, &mut |_| {})
            .await
            .unwrap();
        assert_eq!(result["compactNow"], true);
        assert!(
            tool.prepare_optional(None, json!({"delete":true}), &l)
                .is_err()
        );
    }
}
