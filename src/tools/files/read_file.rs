use crate::tools::{arguments::*, *};
pub struct ReadFile;
impl Tool for ReadFile {
    fn schema(&self) -> Value {
        let (description, properties, required) = (
            "Read a workspace UTF-8 file; Rust automatically discovers and verifies an exact unverified path. Returns bounded excerpts (up to 8 KiB per call). Use nextOffset from a previous result to read a later excerpt when needed; don't repeat unchanged reads.",
            json!({"path":{"type":"string"},"offset":{"type":"integer","minimum":0,"description":"Optional UTF-8 byte offset; reuse nextOffset from the previous excerpt."},"length":{"type":"integer","minimum":4,"maximum":8192,"description":"Optional excerpt byte length, default 8192."}}),
            vec!["path"],
        );
        schema("read_file", description, properties, required)
    }
    fn prepare(&self, w: &Workspace, arguments: Value, _limits: &Limits) -> Result<Prepared> {
        check_size(&arguments)?;
        let action = arguments.clone();
        let payload = arguments;
        {
            let a: ReadArgs = parse(payload.clone())?;
            w.require_reference(&a.path)?;
            if !(4..=8192).contains(&a.length) {
                return Err("Read length must be 4–8192 bytes".into());
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
        p: &'a Prepared,
        _limits: &'a Limits,
        cancel: watch::Receiver<bool>,
        _output: &'a mut (dyn FnMut(Value) + Send),
    ) -> FutureResult<'a, Value> {
        Box::pin(async move {
            if *cancel.borrow() {
                return Err("Cancelled".into());
            }
            {
                let a: ReadArgs = parse(p.payload.clone())?;
                w.excerpt(&a.path, a.offset, a.length)
            }
        })
    }
}
