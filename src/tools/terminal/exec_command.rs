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
    #[serde(default)]
    read_only: bool,
    #[serde(default)]
    network: bool,
    #[serde(default)]
    git_metadata: bool,
    #[serde(default = "yield_default")]
    yield_time_ms: u64,
}
fn yield_default() -> u64 {
    1000
}
impl Tool for ExecCommand {
    fn settle_for_revision(&self) -> FutureResult<'_, ()> {
        Box::pin(self.sessions.settle_for_revision())
    }
    fn schema(&self) -> Value {
        let mut properties = json!({"executable":{"type":"string","description":"Executable name or path."},"arguments":{"type":"array","items":{"type":"string"}},"directory":{"type":"string","description":"Workspace directory; default ."}});
        let description = if self.legacy {
            properties["executable"]["enum"] = json!(["cargo"]);
            "Legacy sandboxed Cargo verification. For general commands use exec_command."
        } else {
            properties["yield_time_ms"] = json!({"type":"integer","minimum":0,"maximum":1000});
            properties["read_only"] =
                json!({"type":"boolean","description":"Block workspace writes."});
            properties["network"] =
                json!({"type":"boolean","description":"Allow outbound network."});
            properties["git_metadata"] =
                json!({"type":"boolean","description":"Allow Git metadata access."});
            "Run approved argv in the workspace sandbox. Use apply_patch for text edits. Poll with write_stdin."
        };
        let mut schema = schema(
            if self.legacy {
                "run_command"
            } else {
                "exec_command"
            },
            description,
            properties,
            vec!["executable", "arguments"],
        );
        schema["function"]["parameters"]["examples"] =
            json!([{"executable":"cargo","arguments":["test"],"directory":"."}]);
        schema
    }
    fn prepare(&self, w: &Workspace, a: Value, l: &Limits) -> Result<Prepared> {
        check_size(&a)?;
        if self.legacy
            && ["yield_time_ms", "read_only", "network", "git_metadata"]
                .iter()
                .any(|key| a.get(key).is_some())
        {
            return Err(
                "Legacy run_command only accepts executable, arguments and directory".into(),
            );
        }
        let args: Args = parse(a)?;
        if args.yield_time_ms > 1000 {
            return Err("Invalid arguments: yield_time_ms must be 0–1000".into());
        }
        let mut spec = if self.legacy {
            CommandSpec::validate(&args.executable, &args.arguments, &args.directory, w)
        } else {
            CommandSpec::general(&args.executable, &args.arguments, &args.directory, w)
        }
        .map_err(|error| format!("Invalid arguments: {error}"))?;
        spec.read_only = args.read_only;
        spec.network = args.network;
        if args.git_metadata {
            spec.with_git_metadata(w)?;
        }
        Ok(Prepared {
            action: spec.action(w, l.command_seconds),
            payload: json!({"spec":spec,"yield_time_ms":args.yield_time_ms}),
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
            let spec: CommandSpec = parse(p.payload["spec"].clone())?;
            let yield_time_ms: u64 = parse(p.payload["yield_time_ms"].clone())?;
            if self.legacy {
                command::run(&spec, w, l.command_seconds, c, o).await
            } else {
                self.sessions
                    .start(spec, w, l.command_seconds, yield_time_ms, c)
                    .await
            }
        })
    }
}
