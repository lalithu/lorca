use crate::tools::{
    arguments::{dot, parse},
    terminal::{
        command::{self, CommandSpec},
        sessions::Sessions,
    },
    *,
};
pub struct ExecCommand {
    pub(super) sessions: Arc<Sessions>,
    pub(super) legacy: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    executable: String,
    arguments: Vec<String>,
    #[serde(default = "dot")]
    directory: String,
    #[serde(default = "yield_default")]
    yield_time_ms: u64,
}
fn yield_default() -> u64 {
    1000
}
impl Tool for ExecCommand {
    fn schema(&self) -> Value {
        let mut properties = json!({"executable":{"type":"string","enum":["cargo"],"description":"Executable name only; do not include it in arguments."},"arguments":{"type":"array","items":{"type":"string"},"description":"A JSON array such as [\"test\"], never a serialized string or shell command."},"directory":{"type":"string","description":"Workspace-relative directory; use . for the workspace root, never cd or an absolute path."}});
        if !self.legacy {
            properties["yield_time_ms"] = json!({"type":"integer","minimum":0,"maximum":1000});
        }
        let mut schema = schema(
            if self.legacy {
                "run_command"
            } else {
                "exec_command"
            },
            "Request approval for sandboxed cargo test/check, fmt --check, or clippy --all-targets -- -D warnings. No shell or arbitrary commands. exec_command yields a task-scoped sessionId when still running; poll with write_stdin. Commands retain a hard timeout.",
            properties,
            vec!["executable", "arguments"],
        );
        schema["function"]["parameters"]["examples"] =
            json!([{"executable":"cargo","arguments":["test"],"directory":"."}]);
        schema
    }
    fn prepare(&self, w: &Workspace, a: Value, l: &Limits) -> Result<Prepared> {
        check_size(&a)?;
        if self.legacy && a.get("yield_time_ms").is_some() {
            return Err("run_command does not accept yield_time_ms".into());
        }
        let args: Args = parse(a.clone())?;
        if args.yield_time_ms > 1000 {
            return Err("yield_time_ms must be 0–1000".into());
        }
        let spec = CommandSpec::validate(&args.executable, &args.arguments, &args.directory, w)
            .map_err(|error| format!("Invalid arguments: {error}"))?;
        Ok(Prepared {
            action: spec.action(w, l.command_seconds),
            payload: a,
            approval: true,
        })
    }
    fn execute<'a>(
        &'a self,
        w: &'a Workspace,
        p: &'a Prepared,
        l: &'a Limits,
        c: watch::Receiver<bool>,
        o: &'a mut (dyn FnMut(Value) + Send),
    ) -> FutureResult<'a, Value> {
        Box::pin(async move {
            let args: Args = parse(p.payload.clone())?;
            let spec =
                CommandSpec::validate(&args.executable, &args.arguments, &args.directory, w)?;
            if self.legacy {
                command::run(&spec, w, l.command_seconds, c, o).await
            } else {
                self.sessions
                    .start(spec, w, l.command_seconds, args.yield_time_ms, c)
                    .await
            }
        })
    }
}
