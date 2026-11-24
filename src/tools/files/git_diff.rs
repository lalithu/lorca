use crate::tools::{
    arguments::{dot, parse},
    terminal::command::{self, CommandSpec},
    *,
};

pub struct GitDiff;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    #[serde(default = "dot")]
    directory: String,
    #[serde(default)]
    staged: bool,
}

impl Tool for GitDiff {
    fn schema(&self) -> Value {
        schema(
            "git_diff",
            "Inspect tracked changes without external diff programs. Use staged: true for the index; default is unstaged. Use exec_command for status and history.",
            json!({"directory":{"type":"string"},"staged":{"type":"boolean"}}),
            vec![],
        )
    }
    fn prepare(&self, w: &Workspace, arguments: Value, _: &Limits) -> Result<Prepared> {
        check_size(&arguments)?;
        let args: Args = parse(arguments)?;
        let (spec, approval) = command::prepare_git_diff(w, &args.directory, args.staged)?;
        Ok(Prepared {
            action: spec.action(w, 10),
            payload: json!(spec),
            approval,
        })
    }
    fn execute<'a>(
        &'a self,
        w: &'a Workspace,
        p: &'a Prepared,
        _: &'a Limits,
        cancel: watch::Receiver<bool>,
        output: &'a mut (dyn FnMut(Value) + Send),
    ) -> FutureResult<'a, Value> {
        Box::pin(async move {
            let spec: CommandSpec = parse(p.payload.clone())?;
            command::run(&spec, w, 10, cancel, output).await
        })
    }
}
