//! Schemas describe capabilities; handlers prepare exact actions; the gate grants access.
pub mod agent;
pub mod files;
pub mod terminal;
pub mod web;
use crate::{
    inference::{Call, Model, Result},
    permissions::Access,
    storage::Repository,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub struct Context {
    pub cwd: PathBuf,
    pub repository: Repository,
    pub conversation: String,
    pub terminal: terminal::Terminal,
    pub model: Model,
}
pub struct Prepared {
    pub action: Value,
    pub access: Access,
    pub operation: Operation,
}
pub enum Operation {
    Read {
        path: PathBuf,
        offset: usize,
        limit: usize,
    },
    List {
        path: PathBuf,
    },
    Search {
        path: PathBuf,
        query: String,
        content: bool,
    },
    Edit {
        changes: Vec<files::Change>,
    },
    Command {
        executable: String,
        arguments: Vec<String>,
        directory: PathBuf,
        timeout: Option<u64>,
    },
    Poll {
        session: String,
        chars: Option<String>,
        close: bool,
        wait: u64,
    },
    Open {
        path: PathBuf,
        application: Option<String>,
    },
    Web {
        url: String,
        query: Option<String>,
    },
    Agent {
        name: String,
        args: Value,
    },
    Image {
        path: PathBuf,
        question: String,
    },
}
pub fn schema(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({"type":"function","function":{"name":name,"description":description,"parameters":{"type":"object","properties":properties,"required":required,"additionalProperties":false}}})
}
pub fn schemas() -> Vec<Value> {
    let mut v = files::schemas();
    v.extend(terminal::schemas());
    v.extend(web::schemas());
    v.extend(agent::schemas());
    v.push(schema(
        "view_image",
        "Read a local image with the selected model. Requires a vision-capable model.",
        json!({"path":{"type":"string"},"question":{"type":"string"}}),
        &["path"],
    ));
    v
}
pub fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Supply {key}"))
}
pub fn path(cwd: &Path, input: &str) -> Result<PathBuf> {
    let p = if input == "~" || input.starts_with("~/") {
        PathBuf::from(std::env::var_os("HOME").ok_or("HOME is unavailable")?)
            .join(input.strip_prefix("~/").unwrap_or(""))
    } else {
        PathBuf::from(input)
    };
    let p = if p.is_absolute() { p } else { cwd.join(p) };
    if p.exists() {
        return p.canonicalize().map_err(|e| e.to_string());
    }
    let parent = p
        .parent()
        .ok_or("Missing parent directory")?
        .canonicalize()
        .map_err(|e| format!("Parent directory: {e}"))?;
    Ok(parent.join(p.file_name().ok_or("Missing filename")?))
}
fn validate(schema: &Value, value: &Value) -> Result<()> {
    let matches = match schema["type"].as_str() {
        Some("object") => value.is_object(),
        Some("string") => value.is_string(),
        Some("array") => value.is_array(),
        Some("integer") => value.as_i64().is_some() || value.as_u64().is_some(),
        Some("boolean") => value.is_boolean(),
        _ => true,
    };
    if !matches {
        return Err(format!(
            "Invalid argument type; expected {}",
            schema["type"]
        ));
    }
    if let Some(variants) = schema["enum"].as_array()
        && !variants.contains(value)
    {
        return Err("Value is outside the allowed enum".into());
    }
    if let Some(o) = value.as_object() {
        for k in schema["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !o.contains_key(k) {
                return Err(format!("Missing argument {k}"));
            }
        }
        for (k, v) in o {
            if let Some(s) = schema["properties"].get(k) {
                validate(s, v)?
            } else if schema["additionalProperties"] == false {
                return Err(format!("Unknown argument {k}"));
            }
        }
    }
    if let Some(a) = value.as_array() {
        for v in a {
            validate(&schema["items"], v)?;
        }
    }
    Ok(())
}
pub async fn prepare(call: &Call, c: &Context) -> Result<Prepared> {
    if call.arguments.to_string().len() > 128 * 1024 {
        return Err("Tool arguments exceed 128 KiB; use smaller edits".into());
    }
    let s = schemas()
        .into_iter()
        .find(|s| s["function"]["name"] == call.name)
        .ok_or_else(|| format!("Unknown tool {}", call.name))?;
    validate(&s["function"]["parameters"], &call.arguments)
        .map_err(|e| format!("{e}. Expected schema: {}", s["function"]["parameters"]))?;
    match call.name.as_str() {
        "read_file" | "list_directory" | "search_files" | "apply_patch" | "open_path" => {
            files::prepare(call, c).await
        }
        "exec_command" | "write_stdin" | "git_diff" => terminal::prepare(call, c),
        "fetch_url" | "web_search" => web::prepare(call),
        "view_image" => {
            let p = path(&c.cwd, text(&call.arguments, "path")?)?;
            Ok(Prepared {
                action: json!({"path":p,"scope":"Read this image using the selected model"}),
                access: Access::Read(p.clone()),
                operation: Operation::Image {
                    path: p,
                    question: call.arguments["question"]
                        .as_str()
                        .unwrap_or("Describe the image accurately.")
                        .into(),
                },
            })
        }
        _ => agent::prepare(call, c),
    }
}
pub async fn execute(p: Prepared, c: &Context) -> Result<Value> {
    match p.operation {
        op @ (Operation::Read { .. }
        | Operation::List { .. }
        | Operation::Search { .. }
        | Operation::Edit { .. }
        | Operation::Open { .. }) => files::execute(op).await,
        Operation::Command {
            executable,
            arguments,
            directory,
            timeout,
        } => {
            c.terminal
                .start(&executable, &arguments, &directory, timeout)
                .await
        }
        Operation::Poll {
            session,
            chars,
            close,
            wait,
        } => c.terminal.poll(&session, chars, close, wait).await,
        Operation::Web { url, query } => web::execute(&url, query).await,
        Operation::Agent { name, args } => agent::execute(&name, &args, c).await,
        Operation::Image { path, question } => {
            use base64::Engine;
            let bytes = tokio::fs::read(&path).await.map_err(|e| e.to_string())?;
            if bytes.len() > 8 * 1024 * 1024 {
                return Err("Image exceeds 8 MiB".into());
            }
            let format = if bytes.starts_with(b"\x89PNG") {
                "png"
            } else if bytes.starts_with(&[0xff, 0xd8]) {
                "jpeg"
            } else {
                return Err("Use PNG or JPEG".into());
            };
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            let mut model = c.model.clone();
            model.effort = "off".into();
            let _ = format; // Both providers accept encoded PNG/JPEG; no local raster conversion.
            let image = encoded;
            let turn = model
                .generate(
                    &[json!({"role":"user","content":question,"images":[image]})],
                    &[],
                    1024,
                    &|_| {},
                )
                .await?;
            Ok(json!({"ok":true,"path":path,"description":turn.message["content"]}))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_arguments() {
        let s = schemas()
            .into_iter()
            .find(|s| s["function"]["name"] == "read_file")
            .unwrap();
        assert!(
            validate(
                &s["function"]["parameters"],
                &json!({"path":"a","approved":true})
            )
            .is_err()
        );
    }
}
