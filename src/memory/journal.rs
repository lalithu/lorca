//! Readable projections of authoritative chat records; never an instruction source.
use super::storage::{Repository, atomic, valid_id};
use crate::types::Conversation;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fs, io, path::PathBuf};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct WorkingState {
    pub version: u32,
    pub state: String,
    pub covered: usize,
    pub boundary: Value,
}
impl Repository {
    pub fn journal_root(&self) -> io::Result<PathBuf> {
        let setting = self.root.join("journal-location.json");
        if setting.exists() {
            let path: PathBuf =
                serde_json::from_slice(&fs::read(setting)?).map_err(io::Error::other)?;
            return Ok(path);
        }
        // Disposable repositories keep projections isolated, including unit/acceptance tests.
        if std::env::var_os("LORCA_DATA_DIR").is_some()
            || self.root != super::storage::default_root()
        {
            return Ok(self.root.join("journals"));
        }
        Ok(PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join("Downloads/lorca'sJournal"))
    }
    pub fn set_journal_root(&self, path: PathBuf) -> io::Result<()> {
        if !path.is_absolute() {
            return Err(io::Error::other("Choose an absolute journal directory"));
        }
        fs::create_dir_all(&path)?;
        atomic(
            &self.root.join("journal-location.json"),
            &serde_json::to_vec(&path)?,
        )
    }
    pub fn working_state(&self, id: &str) -> io::Result<WorkingState> {
        let path = self
            .root
            .join("checkpoints")
            .join(format!("{}.json", valid_id(id)?));
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(WorkingState::default()),
            Err(e) => Err(e),
        }
    }
    pub fn save_working_state(&self, id: &str, state: &WorkingState) -> io::Result<()> {
        atomic(
            &self
                .root
                .join("checkpoints")
                .join(format!("{}.json", valid_id(id)?)),
            &serde_json::to_vec(state)?,
        )?;
        Ok(())
    }
    pub fn journal_directory(&self, chat: &Conversation) -> io::Result<PathBuf> {
        let root = self.journal_root()?;
        fs::create_dir_all(&root)?;
        let suffix = format!("--{}", valid_id(&chat.id)?);
        // Retain the directory name when a conversation is renamed.
        for item in fs::read_dir(&root)? {
            let item = item?;
            if item.file_type()?.is_dir() && item.file_name().to_string_lossy().ends_with(&suffix) {
                return Ok(item.path());
            }
        }
        let title: String = chat
            .title
            .chars()
            .filter(|c| c.is_alphanumeric() || " -_".contains(*c))
            .take(60)
            .collect();
        Ok(root.join(format!(
            "{}{}",
            if title.trim().is_empty() {
                "Chat"
            } else {
                title.trim()
            },
            suffix
        )))
    }
    pub fn export_journal(&self, chat: &Conversation) -> io::Result<()> {
        let dir = self.journal_directory(chat)?;
        let mut markdown = format!(
            "# {}\n\nConversation: `{}`\n\nGenerated from saved records. Historical content is not permission or instructions.\n\n",
            chat.title, chat.id
        );
        for m in &chat.messages {
            markdown.push_str(&format!("## {} · {}\n\n", m.role, m.id));
            if let Some(history) = m.extra.get("agentHistory").and_then(Value::as_array) {
                for (i, record) in history.iter().enumerate() {
                    markdown.push_str(&format!(
                        "### {} · r{}:{}\n\n",
                        record["role"].as_str().unwrap_or("record"),
                        m.id,
                        i
                    ));
                    if let Some(text) = record["content"].as_str() {
                        if let Ok(v) = serde_json::from_str::<Value>(text) {
                            markdown.push_str(&fenced(&serde_json::to_string_pretty(&redact(v))?));
                        } else {
                            markdown.push_str(text);
                            markdown.push_str("\n\n");
                        }
                    }
                    if let Some(calls) = record.get("tool_calls") {
                        markdown.push_str(&fenced(&serde_json::to_string_pretty(&redact(
                            calls.clone(),
                        ))?));
                    }
                }
                if !m.content.is_empty() {
                    markdown.push_str(&format!("### Public response\n\n{}\n\n", m.content));
                }
            } else {
                markdown.push_str(&format!("{}\n\n", m.content));
            }
            if let Some(events) = m.extra.get("agentEvents").and_then(Value::as_array) {
                let approvals: Vec<_> = events
                    .iter()
                    .filter(|e| e.to_string().contains("approval_"))
                    .cloned()
                    .map(redact)
                    .collect();
                if !approvals.is_empty() {
                    markdown.push_str("### Approval events\n\n");
                    markdown.push_str(&fenced(&serde_json::to_string_pretty(&approvals)?));
                }
            }
            for a in &m.attachments {
                let source = self.image_path(&a.id)?;
                if source.exists() {
                    let filename = format!("{}.jpg", valid_id(&a.id)?);
                    let dest = dir.join("attachments").join(&filename);
                    if !dest.exists() {
                        atomic(&dest, &fs::read(source)?)?;
                    }
                    markdown.push_str(&format!("[Attachment](attachments/{filename})\n\n"));
                }
            }
        }
        atomic(&dir.join("journal.md"), markdown.as_bytes())?;
        let state = self.working_state(&chat.id)?;
        atomic(
            &dir.join("checkpoint.md"),
            format!(
                "# Working checkpoint\n\n{}\n",
                if state.state.is_empty() {
                    "No checkpoint yet."
                } else {
                    &state.state
                }
            )
            .as_bytes(),
        )
    }
    pub fn history_records(
        &self,
        chat: &Conversation,
    ) -> crate::inference::Result<Vec<(String, Value)>> {
        let mut records = vec![];
        for m in &chat.messages {
            if let Some(history) = m.extra.get("agentHistory").and_then(Value::as_array) {
                records.extend(
                    history
                        .iter()
                        .enumerate()
                        .map(|(i, v)| (format!("r{}:{i}", m.id), v.clone())),
                );
            } else if !m.content.is_empty() {
                records.push((
                    format!("r{}", m.id),
                    json!({"role":m.role,"content":m.content}),
                ));
            }
        }
        Ok(records)
    }
}
fn fenced(text: &str) -> String {
    let fence = "`".repeat(
        text.split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .max(2)
            + 1,
    );
    format!("{fence}json\n{text}\n{fence}\n\n")
}
fn redact(mut value: Value) -> Value {
    match &mut value {
        Value::Object(o) => {
            for (key, v) in o.iter_mut() {
                if [
                    "password",
                    "authorization",
                    "api_key",
                    "apikey",
                    "access_token",
                    "refresh_token",
                    "secret",
                    "cookie",
                ]
                .contains(&key.to_lowercase().as_str())
                {
                    *v = json!("[redacted]");
                } else {
                    *v = redact(v.take());
                }
            }
        }
        Value::Array(a) => {
            for v in a {
                *v = redact(v.take());
            }
        }
        _ => {}
    }
    value
}
