use crate::tools::{arguments::*, *};
pub struct SearchFiles;
impl Tool for SearchFiles {
    fn schema(&self) -> Value {
        let (description, properties, required) = (
            "Search filenames and UTF-8 contents for a literal query. Skips build and hidden folders; bounded scan.",
            json!({"query":{"type":"string","description":"Literal substring, NOT a regex. Example: reqwest"},"path":{"type":"string"}}),
            vec!["query"],
        );
        schema("search_files", description, properties, required)
    }
    fn prepare(&self, _w: &Workspace, arguments: Value, _limits: &Limits) -> Result<Prepared> {
        check_size(&arguments)?;
        let action = arguments.clone();
        let payload = arguments;
        {
            let a: SearchArgs = parse(payload.clone())?;
            Workspace::path(&a.path)?;
            if a.query.is_empty() || a.query.len() > 200 {
                return Err("Search query must contain 1–200 bytes".into());
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
                let a: SearchArgs = parse(p.payload.clone())?;
                search(w, &a, &cancel)
            }
        })
    }
}

fn search(w: &Workspace, a: &SearchArgs, cancel: &watch::Receiver<bool>) -> Result<Value> {
    let start = std::time::Instant::now();
    let mut pending = vec![a.path.clone()];
    let mut matches = vec![];
    let mut scanned = 0;
    let mut truncated = false;
    'scan: while let Some(path) = pending.pop() {
        if *cancel.borrow() {
            return Err("Cancelled".into());
        }
        let dir = w.directory(&path)?;
        for item in dir.entries().map_err(|e| e.to_string())? {
            if scanned >= 800 || matches.len() >= 30 || start.elapsed().as_secs() >= 2 {
                truncated = true;
                break 'scan;
            }
            scanned += 1;
            let entry = item.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || matches!(name.as_str(), "target" | "node_modules" | "build")
            {
                continue;
            }
            let p = if path == "." {
                name.clone()
            } else {
                format!("{path}/{name}")
            };
            let ty = entry.file_type().map_err(|e| e.to_string())?;
            if ty.is_symlink() {
                continue;
            }
            if name.contains(&a.query)
                && let Ok(reference) = w.remember(&p)
            {
                matches.push(json!({"path":reference}));
            }
            if ty.is_dir() {
                pending.push(p);
            } else if ty.is_file()
                && let Ok(text) = w.read(&p)
            {
                for (i, line) in text.lines().enumerate() {
                    if line.contains(&a.query) {
                        let reference = w.remember(&p)?;
                        matches.push(json!({"path":reference,"line":i+1,"text":bounded(line,240)}));
                        if matches.len() >= 30 {
                            truncated = true;
                            break 'scan;
                        }
                    }
                }
            }
        }
    }
    Ok(
        json!({"ok":true,"matches":matches,"scanned":scanned,"truncated":truncated,"note":"Literal substring search; regex operators are not interpreted"}),
    )
}
