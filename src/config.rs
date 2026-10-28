use std::{env, fs, io, path::PathBuf};

use serde_json::{Value, json};

pub const HELP: &str = "lorca — chat with a local Ollama model

Usage: lorca [--model NAME] [--host URL] [--config PATH]
       lorca main
       lorca rebuild

  lorca          Start terminal chat
  lorca main     Open the macOS dashboard and Mini
  lorca rebuild  Rebuild/install the CLI and macOS app, then open the dashboard

  --model NAME   Override the saved model for this session
  --host URL     Override the saved Ollama address for this session
  --config PATH  Use a different JSON configuration file
  -h, --help     Show this help

Defaults: qwen3.5:9b at http://127.0.0.1:11434
Configuration: $XDG_CONFIG_HOME/lorca/config.json or ~/.config/lorca/config.json";

pub struct Config {
    pub model: String,
    pub base_url: String,
}

impl Config {
    pub fn load() -> io::Result<Option<Self>> {
        let args: Vec<String> = env::args().skip(1).collect();
        if args.iter().any(|arg| arg == "--help" || arg == "-h") {
            println!("{HELP}");
            return Ok(None);
        }
        let mut model = None;
        let mut host = None;
        let mut path = None;
        let mut args = args.iter();
        while let Some(flag) = args.next() {
            let target = match flag.as_str() {
                "--model" => &mut model,
                "--host" => &mut host,
                "--config" => &mut path,
                _ => {
                    return Err(invalid(format!(
                        "Unknown option: {flag}. Use lorca --help."
                    )));
                }
            };
            let value = args
                .next()
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| invalid(format!("{flag} needs a value")))?;
            *target = Some(value.to_string());
        }

        let path = match path {
            Some(path) => PathBuf::from(path),
            None => default_path()?,
        };
        let saved = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<Value>(&text)
                .map_err(|error| invalid(format!("{}: {error}", path.display())))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let defaults = json!({
                    "model": "qwen3.5:9b",
                    "base_url": "http://127.0.0.1:11434"
                });
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                fs::write(
                    &path,
                    format!("{}\n", serde_json::to_string_pretty(&defaults)?),
                )?;
                defaults
            }
            Err(error) => return Err(error),
        };
        Self::resolve(&saved, model, host).map(Some)
    }

    fn resolve(saved: &Value, model: Option<String>, host: Option<String>) -> io::Result<Self> {
        if !saved.is_object() {
            return Err(invalid("Configuration must be a JSON object"));
        }
        let model = model.unwrap_or(read_string(saved, "model", "qwen3.5:9b")?);
        let base_url = host.unwrap_or(read_string(saved, "base_url", "http://127.0.0.1:11434")?);
        if model.trim().is_empty() || model.chars().any(char::is_control) {
            return Err(invalid("Model name must be nonempty text"));
        }
        let url = reqwest::Url::parse(&base_url)
            .map_err(|_| invalid("Ollama host must be an http:// or https:// URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid(
                "Ollama host must be an http:// or https:// URL without credentials, query, or fragment",
            ));
        }
        Ok(Self {
            model,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }
}

fn default_path() -> io::Result<PathBuf> {
    if let Some(path) = env::var_os("XDG_CONFIG_HOME").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path).join("lorca/config.json"));
    }
    let home = env::var_os("HOME").ok_or_else(|| invalid("Set HOME or use --config PATH"))?;
    Ok(PathBuf::from(home).join(".config/lorca/config.json"))
}

fn read_string(saved: &Value, key: &str, default: &str) -> io::Result<String> {
    match saved.get(key) {
        None => Ok(default.to_string()),
        Some(Value::String(value)) => Ok(value.clone()),
        _ => Err(invalid(format!(
            "Configuration field {key} must be a string"
        ))),
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_override_saved_settings() {
        let saved = json!({"model": "saved", "base_url": "http://localhost:1234"});
        let config = Config::resolve(
            &saved,
            Some("override".into()),
            Some("http://other:11434/".into()),
        )
        .unwrap();
        assert_eq!(config.model, "override");
        assert_eq!(config.base_url, "http://other:11434");
        assert_eq!(saved["model"], "saved");
    }

    #[test]
    fn rejects_invalid_configuration() {
        for saved in [
            json!([]),
            json!({"model": 1}),
            json!({"model": ""}),
            json!({"base_url": "file:///tmp/model"}),
        ] {
            assert!(Config::resolve(&saved, None, None).is_err());
        }
    }
}
