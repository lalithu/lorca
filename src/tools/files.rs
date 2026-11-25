use super::*;
use std::fs;
#[derive(Clone)]
pub struct Change {
    pub path: PathBuf,
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
}
pub fn schemas() -> Vec<Value> {
    vec![
        schema(
            "read_file",
            "Read a file at a known path; no discovery prerequisite. Offsets count UTF-8 bytes.",
            json!({"path":{"type":"string"},"offset":{"type":"integer"},"limit":{"type":"integer"}}),
            &["path"],
        ),
        schema(
            "list_directory",
            "List a directory, with a bounded result.",
            json!({"path":{"type":"string"}}),
            &["path"],
        ),
        schema(
            "search_files",
            "Recursively find paths by name, or search file contents. Results are bounded; narrow the path or query if truncated.",
            json!({"path":{"type":"string"},"query":{"type":"string"},"content":{"type":"boolean"}}),
            &["path", "query"],
        ),
        schema(
            "apply_patch",
            "Propose small exact file edits. Use path, old_text, new_text; empty old_text creates a NEW file only. Alternatively use patch with *** Begin Patch, *** Update/Add/Delete File: path, bare @@ hunks and *** End Patch. Read current contents first.",
            json!({"path":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"},"patch":{"type":"string"}}),
            &[],
        ),
        schema(
            "open_path",
            "Open a file or folder and request foreground activation using macOS, optionally with an application such as Visual Studio Code. Only claim the app is foreground when the result reports foreground: true.",
            json!({"path":{"type":"string"},"application":{"type":"string"}}),
            &["path"],
        ),
    ]
}
pub async fn prepare(call: &Call, c: &Context) -> Result<Prepared> {
    let a = &call.arguments;
    if call.name == "apply_patch" {
        let changes = if let Some(patch) = a["patch"].as_str() {
            parse_patch(&c.cwd, patch)?
        } else {
            let p = path(&c.cwd, text(a, "path")?)?;
            let before = read_existing(&p)?;
            let old = a["old_text"].as_str().ok_or("Supply old_text")?;
            let new = a["new_text"].as_str().ok_or("Supply new_text")?;
            let after = if before.is_none() {
                if !old.is_empty() {
                    return Err("New files require empty old_text".into());
                }
                new.into()
            } else {
                let s = std::str::from_utf8(before.as_ref().unwrap())
                    .map_err(|_| "Cannot edit binary files")?;
                if old.is_empty() || s.matches(old).count() != 1 {
                    return Err("old_text must match exactly once. Re-read the file and use a unique excerpt.".into());
                }
                s.replacen(old, new, 1)
            };
            vec![Change {
                path: p,
                before,
                after: Some(after.into_bytes()),
            }]
        };
        if changes.is_empty() {
            return Err("No edits supplied".into());
        }
        let mut seen = std::collections::HashSet::new();
        for x in &changes {
            if !seen.insert(&x.path) {
                return Err("Use one file section per path".into());
            }
            if x.before == x.after {
                return Err("Edit makes no change".into());
            }
        }
        let previews:Vec<_>=changes.iter().map(|x|json!({"path":x.path,"changeKind":if x.after.is_none(){"Delete"}else if x.before.is_none(){"Create"}else{"Edit"},"diff":preview(x)})).collect();
        let action = if previews.len() == 1 {
            previews[0].clone()
        } else {
            json!({"changes":previews,"diff":changes.iter().map(preview).collect::<Vec<_>>().join("\n")})
        };
        return Ok(Prepared {
            action,
            access: Access::Write,
            operation: Operation::Edit { changes },
        });
    }
    let p = path(&c.cwd, text(a, "path")?)?;
    let op = match call.name.as_str() {
        "read_file" => Operation::Read {
            path: p.clone(),
            offset: a["offset"].as_u64().unwrap_or(0) as usize,
            limit: a["limit"].as_u64().unwrap_or(16384).clamp(1, 32768) as usize,
        },
        "list_directory" => Operation::List { path: p.clone() },
        "search_files" => Operation::Search {
            path: p.clone(),
            query: text(a, "query")?.into(),
            content: a["content"] == true,
        },
        "open_path" => Operation::Open {
            path: p.clone(),
            application: a["application"].as_str().map(str::to_owned),
        },
        _ => unreachable!(),
    };
    let open = call.name == "open_path";
    Ok(Prepared {
        action: if open {
            json!({"target":p,"application":a["application"],"scope":"Open this path in a desktop application"})
        } else {
            json!({"target":p,"scope":"Read this path"})
        },
        access: if open {
            Access::Launch
        } else {
            Access::Read(p)
        },
        operation: op,
    })
}
fn read_existing(p: &Path) -> Result<Option<Vec<u8>>> {
    match fs::metadata(p) {
        Ok(m) => {
            if !m.is_file() || m.len() > 2 * 1024 * 1024 {
                return Err("Edit only regular files under 2 MiB".into());
            }
            fs::read(p).map(Some).map_err(|e| e.to_string())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
fn preview(c: &Change) -> String {
    let old = c
        .before
        .as_ref()
        .map(|b| String::from_utf8_lossy(b))
        .unwrap_or_default();
    let new = c
        .after
        .as_ref()
        .map(|b| String::from_utf8_lossy(b))
        .unwrap_or_default();
    format!(
        "--- {}\n+++ {}\n{}{}",
        c.path.display(),
        c.path.display(),
        old.lines().map(|l| format!("-{l}\n")).collect::<String>(),
        new.lines().map(|l| format!("+{l}\n")).collect::<String>()
    )
}
fn parse_patch(cwd: &Path, patch: &str) -> Result<Vec<Change>> {
    let lines: Vec<_> = patch.lines().collect();
    if lines.first() != Some(&"*** Begin Patch") || lines.last() != Some(&"*** End Patch") {
        return Err("Patch requires Begin/End markers".into());
    }
    let mut i = 1;
    let mut out = vec![];
    while i < lines.len() - 1 {
        let header = lines[i];
        let (kind, name) = if let Some(n) = header.strip_prefix("*** Update File: ") {
            ("update", n)
        } else if let Some(n) = header.strip_prefix("*** Add File: ") {
            ("add", n)
        } else if let Some(n) = header.strip_prefix("*** Delete File: ") {
            ("delete", n)
        } else {
            return Err(format!("Invalid patch header {header}"));
        };
        i += 1;
        let p = path(cwd, name)?;
        let before = read_existing(&p)?;
        let mut after = before
            .as_ref()
            .map(|b| {
                String::from_utf8(b.clone()).map_err(|_| "Cannot edit binary files".to_string())
            })
            .transpose()?
            .unwrap_or_default();
        if kind == "add" && before.is_some() {
            return Err("Add target already exists".into());
        }
        if kind != "add" && before.is_none() {
            return Err("Edit target does not exist".into());
        }
        if kind == "add" {
            while i < lines.len() - 1 && !lines[i].starts_with("*** ") {
                after.push_str(
                    lines[i]
                        .strip_prefix('+')
                        .ok_or("Add lines must start with +")?,
                );
                after.push('\n');
                i += 1;
            }
        }
        if kind == "update" {
            let mut hunks = 0;
            while i < lines.len() - 1 && !lines[i].starts_with("*** ") {
                if lines[i] != "@@" {
                    return Err("Use bare @@ before each hunk".into());
                }
                i += 1;
                let mut old = vec![];
                let mut new = vec![];
                while i < lines.len() - 1 && lines[i] != "@@" && !lines[i].starts_with("*** ") {
                    let l = lines[i];
                    match l.as_bytes().first() {
                        Some(b' ') => {
                            old.push(&l[1..]);
                            new.push(&l[1..]);
                        }
                        Some(b'-') => old.push(&l[1..]),
                        Some(b'+') => new.push(&l[1..]),
                        _ => return Err("Hunk lines must start with space, + or -".into()),
                    }
                    i += 1;
                }
                let old = old.join("\n");
                let new = new.join("\n");
                if old.is_empty() || after.matches(&old).count() != 1 {
                    return Err(
                        "Patch context must match exactly once; re-read and use unique context"
                            .into(),
                    );
                }
                after = after.replacen(&old, &new, 1);
                hunks += 1;
            }
            if hunks == 0 {
                return Err("Update requires a hunk".into());
            }
        }
        out.push(Change {
            path: p,
            before,
            after: if kind == "delete" {
                None
            } else {
                Some(after.into_bytes())
            },
        });
    }
    Ok(out)
}
pub async fn execute(op: Operation) -> Result<Value> {
    if let Operation::Open { path, application } = op {
        // The bundled native executable uses NSWorkspace and confirms activation.
        // No fallback to shell activation or automation permissions.
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let sibling = executable
            .parent()
            .ok_or("Missing executable directory")?
            .join("lorca");
        let native =
            if executable.file_name().is_some_and(|n| n == "lorca-core") && sibling.is_file() {
                sibling
            } else {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("macos/build/lorca.app/Contents/MacOS/lorca")
            };
        if !native.is_file() {
            return Err("Native app-opening helper is missing. Run lorca rebuild.".into());
        }
        let mut cmd = tokio::process::Command::new(native);
        cmd.arg("--lorca-open").arg(&path).kill_on_drop(true);
        if let Some(app) = application {
            cmd.arg(app);
        }
        let output = tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output())
            .await
            .map_err(
                |_| "Native application opening timed out; check the desktop before retrying.",
            )?
            .map_err(|e| e.to_string())?;
        let result: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
            format!(
                "Native app-opening helper returned no result ({}). Rebuild the app. {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )
        })?;
        if !output.status.success() || result["ok"] != true {
            return Err(result["error"]
                .as_str()
                .unwrap_or("Application launch failed")
                .into());
        }
        return Ok(result);
    }
    tokio::task::spawn_blocking(move||match op{
        Operation::Read{path,offset,limit}=>{
            use std::io::{Read,Seek,SeekFrom};let mut f=fs::File::open(&path).map_err(|e|e.to_string())?;let total=f.metadata().map_err(|e|e.to_string())?.len();f.seek(SeekFrom::Start(offset as u64)).map_err(|e|e.to_string())?;let mut b=vec![];f.take(limit as u64).read_to_end(&mut b).map_err(|e|e.to_string())?;
            let next=offset+b.len();Ok(json!({"ok":true,"path":path,"offset":offset,"totalBytes":total,"nextOffset":next,"truncated":(next as u64)<total,"content":String::from_utf8_lossy(&b)}))
        }
        Operation::List{path}=>{
            let mut entries=vec![];for entry in fs::read_dir(&path).map_err(|e|e.to_string())?.take(201){let e=entry.map_err(|e|e.to_string())?;let t=e.file_type().map_err(|e|e.to_string())?;entries.push(json!({"name":e.file_name().to_string_lossy(),"path":e.path(),"kind":if t.is_symlink(){"symlink"}else if t.is_dir(){"directory"}else{"file"}}));}
            let truncated=entries.len()>200;entries.truncate(200);Ok(json!({"ok":true,"path":path,"entries":entries,"truncated":truncated}))
        }
        Operation::Search{path,query,content}=>{
            let mut queue=vec![path];let mut matches=vec![];let mut visited=0;let mut truncated=false;
            while let Some(p)=queue.pop(){visited+=1;if visited>20000||matches.len()>=100{truncated=true;break}let m=match fs::symlink_metadata(&p){Ok(m)=>m,Err(_)=>continue};if m.is_symlink(){continue}
                if m.is_dir(){if let Ok(entries)=fs::read_dir(&p){for e in entries.flatten(){if ![".git","target","node_modules",".build"].contains(&e.file_name().to_string_lossy().as_ref()){queue.push(e.path());}}}continue}
                if !content {if p.file_name().is_some_and(|n|n.to_string_lossy().to_lowercase().contains(&query.to_lowercase())){matches.push(json!({"path":p}));}}
                else if m.len()<2*1024*1024 && let Ok(s)=fs::read_to_string(&p){for (i,line) in s.lines().enumerate(){if line.contains(&query){matches.push(json!({"path":p,"line":i+1,"text":line.chars().take(300).collect::<String>()}));if matches.len()>=100{truncated=true;break}}}}
            }Ok(json!({"ok":true,"matches":matches,"truncated":truncated,"skippedDirectories":[".git","target","node_modules",".build"]}))
        }
        Operation::Edit{changes}=>{
            for c in &changes {if super::path(Path::new("/"),c.path.to_str().ok_or("Invalid path")?)?!=c.path||read_existing(&c.path)?!=c.before{return Err(format!("{} changed since approval; read it again",c.path.display()))}}
            let mut applied=vec![];for c in &changes {
                let result=write_change(c,&c.after);
                if let Err(e)=result {for old in applied.into_iter().rev(){let old:&Change=old;let _=write_change(old,&old.before);}return Err(format!("Edit failed; attempted rollback: {e}"))}applied.push(c);
            }
            Ok(json!({"ok":true,"verified":true,"changes":changes.iter().map(|c|json!({"path":c.path,"bytes":c.after.as_ref().map(Vec::len),"verified":true})).collect::<Vec<_>>()}))
        }
        _=>unreachable!()
    }).await.map_err(|e|e.to_string())?
}
fn write_change(c: &Change, bytes: &Option<Vec<u8>>) -> Result<()> {
    if let Some(bytes) = bytes {
        let mode = fs::metadata(&c.path).ok().map(|m| m.permissions());
        crate::storage::atomic(&c.path, bytes).map_err(|e| e.to_string())?;
        if let Some(p) = mode {
            fs::set_permissions(&c.path, p).map_err(|e| e.to_string())?
        }
        if fs::read(&c.path).map_err(|e| e.to_string())? != *bytes {
            return Err("Readback mismatch".into());
        }
    } else {
        fs::remove_file(&c.path).map_err(|e| e.to_string())?
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patch_is_small_and_exact() {
        let p = std::env::temp_dir().join(crate::types::id());
        fs::create_dir(&p).unwrap();
        fs::write(p.join("a"), "hello\n").unwrap();
        let c = parse_patch(
            &p,
            "*** Begin Patch\n*** Update File: a\n@@\n-hello\n+world\n*** End Patch",
        )
        .unwrap();
        assert_eq!(c[0].after.as_deref(), Some(b"world\n".as_slice()));
        assert!(
            parse_patch(
                &p,
                "*** Begin Patch\n*** Update File: a\n@@\n-missing\n+x\n*** End Patch"
            )
            .is_err()
        );
        fs::remove_dir_all(p).unwrap();
    }
}
