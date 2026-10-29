use crate::tools::{terminal::command, *};
pub struct GitDiff;
impl Tool for GitDiff {
    fn schema(&self) -> Value {
        let (description, properties, required) = (
            "Read tracked working-tree changes with git diff (bounded; no external diff/textconv).",
            json!({}),
            vec![],
        );
        schema("git_diff", description, properties, required)
    }
    fn prepare(&self, _w: &Workspace, arguments: Value, _limits: &Limits) -> Result<Prepared> {
        check_size(&arguments)?;
        let action = arguments.clone();
        let payload = arguments;
        {
            if payload != json!({}) {
                return Err("git_diff takes no arguments".into());
            }
        }
        Ok(Prepared {
            action,
            payload,
            approval: false,
        })
    }
    fn execute<'a>(
        &'a self,
        w: &'a Workspace,
        _p: &'a Prepared,
        _limits: &'a Limits,
        cancel: watch::Receiver<bool>,
        _output: &'a mut (dyn FnMut(Value) + Send),
    ) -> FutureResult<'a, Value> {
        Box::pin(async move {
            if *cancel.borrow() {
                return Err("Cancelled".into());
            }
            command::git_diff(w, cancel).await
        })
    }
}
