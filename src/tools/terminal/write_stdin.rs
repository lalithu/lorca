use crate::tools::{arguments::parse, terminal::sessions::Sessions, *};
pub struct WriteStdin {
    pub(super) sessions: Arc<Sessions>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    session_id: String,
    #[serde(default)]
    chars: String,
    #[serde(default)]
    close_stdin: bool,
    #[serde(default)]
    yield_time_ms: u64,
}
impl Tool for WriteStdin {
    fn schema(&self) -> Value {
        schema(
            "write_stdin",
            "Poll a task-scoped command session or write stdin. Nonempty input and closing stdin require approval; polling does not. Session access never expands command authority.",
            json!({"session_id":{"type":"string"},"chars":{"type":"string","maxLength":4096},"close_stdin":{"type":"boolean"},"yield_time_ms":{"type":"integer","minimum":0,"maximum":1000}}),
            vec!["session_id"],
        )
    }
    fn prepare(&self, _: &Workspace, a: Value, _: &Limits) -> Result<Prepared> {
        check_size(&a)?;
        let args: Args = parse(a.clone())?;
        if args.chars.len() > 4096 || args.yield_time_ms > 1000 {
            return Err("Input limit 4096 bytes; yield_time_ms 0–1000".into());
        }
        self.sessions.require(&args.session_id)?;
        prepared(a, !args.chars.is_empty() || args.close_stdin)
    }
    fn execute<'a>(
        &'a self,
        _: &'a Workspace,
        p: &'a Prepared,
        _: &'a Limits,
        c: watch::Receiver<bool>,
        _: &'a mut (dyn FnMut(Value) + Send),
    ) -> FutureResult<'a, Value> {
        Box::pin(async move {
            let args: Args = parse(p.payload.clone())?;
            self.sessions
                .interact(
                    &args.session_id,
                    args.chars,
                    args.close_stdin,
                    args.yield_time_ms,
                    c,
                )
                .await
        })
    }
}
