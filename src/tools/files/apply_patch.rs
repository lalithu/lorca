use crate::tools::{arguments::*, *};
pub struct ApplyPatch;
impl Tool for ApplyPatch {
    fn schema(&self) -> Value {
        let (description, properties, required) = (
            "Create or edit one UTF-8 file atomically (up to 128 KiB; arguments up to 64 KiB). No Cargo project needed. For a new file use old_text empty; new_text empty creates a truly empty file. Otherwise old_text must match exactly once. User approves the full diff. Never delete files.",
            json!({"path":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"}}),
            vec!["path", "old_text", "new_text"],
        );
        let mut s = schema("apply_patch", description, properties, required);
        s["function"]["parameters"]["examples"] = json!([
            {"path":"README.md","old_text":"","new_text":""}
        ]);
        s
    }
    fn prepare(&self, w: &Workspace, arguments: Value, _limits: &Limits) -> Result<Prepared> {
        check_size(&arguments)?;
        let action;
        let mut payload = arguments;
        {
            let a: PatchArgs = parse(payload)?;
            let old = w.patch_source(&a.path)?;
            if old.is_some() {
                w.require_reference(&a.path)?;
            } else {
                let parent = std::path::Path::new(&a.path)
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(std::path::Path::new("."));
                if parent != std::path::Path::new(".") {
                    w.require_reference(&parent.to_string_lossy())?;
                }
            }
            let new=match &old {
                    None if a.old_text.is_empty() => a.new_text,
                    Some(s) if !a.old_text.is_empty() && s.matches(&a.old_text).count()==1 => s.replacen(&a.old_text,&a.new_text,1),
                    _=>return Err("old_text must match exactly once; empty old_text is only for creating a new file".into()),
                };
            if new.len() > crate::tools::workspace::FILE_LIMIT || old.as_deref() == Some(&new) {
                return Err("Patch is empty or too large".into());
            }
            let diff = full_diff(&a.path, old.as_deref(), &new);
            action = json!({"path":a.path,"diff":diff});
            payload = json!({"path":a.path,"expected":old,"replacement":new});
        }
        Ok(Prepared {
            action,
            payload,
            approval: true,
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
                let path = string(&p.payload, "path")?;
                let replacement = string(&p.payload, "replacement")?;
                let expected: Option<String> = parse(p.payload["expected"].clone())?;
                w.replace(&path, &expected, &replacement)?;
                w.remember(&path)?;
                Ok(json!({"ok":true,"verified":true,"path":path,"bytes":replacement.len()}))
            }
        })
    }
}

fn full_diff(path: &str, old: Option<&str>, new: &str) -> String {
    let mut diff = format!(
        "--- {}\n+++ {path}\n@@ -1,{} +1,{} @@\n",
        if old.is_some() { path } else { "/dev/null" },
        old.unwrap_or("").lines().count(),
        new.lines().count()
    );
    for (prefix, text) in [("-", old.unwrap_or("")), ("+", new)] {
        for line in text.split_inclusive('\n') {
            diff.push_str(prefix);
            diff.push_str(line);
            if !line.ends_with('\n') {
                diff.push_str("\n\\ No newline at end of file\n");
            }
        }
    }
    diff
}
