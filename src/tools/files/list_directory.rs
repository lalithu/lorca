use crate::tools::{arguments::*, *};
pub struct ListDirectory;
impl Tool for ListDirectory {
    fn schema(&self) -> Value {
        let (description, properties, required) = (
            "List immediate entries including hidden names, bounded to 200. An empty untruncated result means this directory is empty. Paths are relative to the workspace; use . for its root.",
            json!({"path":{"type":"string"}}),
            vec!["path"],
        );
        schema("list_directory", description, properties, required)
    }
    fn prepare(&self, _w: &Workspace, arguments: Value, _limits: &Limits) -> Result<Prepared> {
        check_size(&arguments)?;
        let action = arguments.clone();
        let payload = arguments;
        {
            let a: PathArgs = parse(payload.clone())?;
            Workspace::path(&a.path)?;
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
                let a: PathArgs = parse(p.payload.clone())?;
                let dir = w.directory(&a.path)?;
                let mut entries = vec![];
                let mut truncated = false;
                for entry in dir.entries().map_err(|e| e.to_string())? {
                    if entries.len() == 200 {
                        truncated = true;
                        break;
                    }
                    let e = entry.map_err(|e| e.to_string())?;
                    let ty = e.file_type().map_err(|e| e.to_string())?;
                    let name = e.file_name().to_string_lossy().to_string();
                    let path = if a.path == "." {
                        name.clone()
                    } else {
                        format!("{}/{name}", a.path)
                    };
                    let reference = if ty.is_symlink() {
                        None
                    } else {
                        w.remember(&path).ok()
                    };
                    entries.push(json!({"name":name,"path":reference,"kind":if ty.is_dir(){"directory"}else if ty.is_symlink(){"symlink"}else{"file"}}));
                }
                entries.sort_by_key(|e| e["name"].as_str().unwrap_or("").to_string());
                Ok(
                    json!({"ok":true,"path":a.path,"empty":entries.is_empty() && !truncated,"includesHidden":true,"entries":entries,"truncated":truncated}),
                )
            }
        })
    }
}
