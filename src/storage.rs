use crate::types::{Attachment, Conversation, id};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

pub fn default_root() -> PathBuf {
    if let Some(path) = std::env::var_os("LORCA_DATA_DIR") {
        return path.into();
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/lorca")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("lorca")
    }
}
pub fn valid_id(value: &str) -> io::Result<String> {
    uuid::Uuid::parse_str(value)
        .map(|v| v.to_string().to_uppercase())
        .map_err(|_| io::Error::other("Invalid identifier"))
}
pub fn atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Invalid storage path"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".{}.tmp", id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        fs::File::open(parent)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
#[derive(Clone)]
pub struct Repository {
    pub root: PathBuf,
}
impl Repository {
    pub fn new(root: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(root.join("conversations"))?;
        fs::create_dir_all(root.join("attachments"))?;
        // Compatibility, not conversion: keep Foundation dates, UUIDs and image names unchanged.
        // Copy exact original bytes once before this version writes anything.
        let backup = root.join("backups/pre-rust-core");
        if !backup.join("complete").exists() {
            for directory in ["conversations", "attachments"] {
                fs::create_dir_all(backup.join(directory))?;
                for entry in fs::read_dir(root.join(directory))? {
                    let entry = entry?;
                    if entry.file_type()?.is_file()
                        && !backup.join(directory).join(entry.file_name()).exists()
                    {
                        fs::copy(entry.path(), backup.join(directory).join(entry.file_name()))?;
                    }
                }
            }
            if root.join("selected.json").exists() && !backup.join("selected.json").exists() {
                fs::copy(root.join("selected.json"), backup.join("selected.json"))?;
            }
            atomic(&backup.join("complete"), b"1")?;
        }
        Ok(Self { root })
    }
    pub fn load(&self) -> io::Result<(Vec<Conversation>, Option<String>, Option<String>)> {
        let mut chats = vec![];
        let mut warning = None;
        for entry in fs::read_dir(self.root.join("conversations"))? {
            let path = entry?.path();
            if path.extension().is_none_or(|s| s != "json") {
                continue;
            }
            let parsed = fs::read(&path)
                .and_then(|b| serde_json::from_slice::<Conversation>(&b).map_err(io::Error::other));
            match parsed {
                Ok(mut chat)
                    if chat.version == 1
                        && valid_id(&chat.id).is_ok()
                        && path
                            .file_stem()
                            .and_then(|v| v.to_str())
                            .is_some_and(|s| s.eq_ignore_ascii_case(&chat.id)) =>
                {
                    if chat.images().any(|a| valid_id(&a.id).is_err()) {
                        warning=Some("Some saved chats couldn’t be read. Original files were left untouched.".into());
                        continue;
                    }
                    chat.messages.retain(|m| {
                        m.role != "assistant"
                            || !m.content.is_empty()
                            || !m.attachments.is_empty()
                            || m.extra.contains_key("agentHistory")
                            || m.extra.contains_key("agentEvents")
                    });
                    chats.push(chat);
                }
                _ => {
                    warning = Some(
                        "Some saved chats couldn’t be read. Original files were left untouched."
                            .into(),
                    )
                }
            }
        }
        chats.sort_by(|a, b| b.updated_at.total_cmp(&a.updated_at));
        let selected = fs::read(self.root.join("selected.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<String>(&b).ok());
        Ok((chats, selected, warning))
    }
    pub fn save(&self, chat: &Conversation) -> io::Result<()> {
        atomic(
            &self
                .root
                .join("conversations")
                .join(format!("{}.json", valid_id(&chat.id)?)),
            &serde_json::to_vec(chat)?,
        )
    }
    pub fn select(&self, id: &str) -> io::Result<()> {
        valid_id(id)?;
        atomic(&self.root.join("selected.json"), &serde_json::to_vec(id)?)
    }
    pub fn delete(&self, id: &str) -> io::Result<()> {
        fs::remove_file(
            self.root
                .join("conversations")
                .join(format!("{}.json", valid_id(id)?)),
        )
    }
    pub fn cleanup(&self, candidates: &[Attachment]) -> io::Result<()> {
        let (chats, _, warning) = self.load()?;
        if warning.is_some() {
            return Ok(());
        }
        let used: HashSet<_> = chats
            .iter()
            .flat_map(|c| c.images())
            .map(|a| a.id.to_uppercase())
            .collect();
        for a in candidates {
            if !used.contains(&a.id.to_uppercase()) {
                let path = self.image_path(&a.id)?;
                if path.exists() {
                    fs::remove_file(path)?;
                }
            }
        }
        Ok(())
    }
    pub fn image_path(&self, id: &str) -> io::Result<PathBuf> {
        Ok(self
            .root
            .join("attachments")
            .join(format!("{}.jpg", valid_id(id)?)))
    }
    pub fn import_image(&self, data: &str, name: &str) -> Result<Attachment, String> {
        if data.len() > 12 * 1024 * 1024 {
            return Err("Optimized images must be under 8 MB.".into());
        }
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| "Invalid image encoding")?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("Optimized images must be under 8 MB.".into());
        }
        let reader =
            image::ImageReader::with_format(std::io::Cursor::new(&bytes), image::ImageFormat::Jpeg);
        let (width, height) = reader
            .into_dimensions()
            .map_err(|_| "That image is unreadable.")?;
        if width == 0 || height == 0 || width > 4096 || height > 4096 {
            return Err("Image dimensions exceed 4096 pixels.".into());
        }
        // Decode with a fixed allocation budget, rejecting malformed JPEGs before persistence.
        let mut reader =
            image::ImageReader::with_format(std::io::Cursor::new(&bytes), image::ImageFormat::Jpeg);
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(80 * 1024 * 1024);
        reader.limits(limits);
        reader.decode().map_err(|_| "That image is unreadable.")?;
        let attachment = Attachment {
            id: id(),
            name: name.chars().take(200).collect(),
            width,
            height,
        };
        atomic(
            &self.image_path(&attachment.id).map_err(|e| e.to_string())?,
            &bytes,
        )
        .map_err(|e| e.to_string())?;
        Ok(attachment)
    }
    pub fn wire_messages(&self, chat: &Conversation) -> Result<Vec<Value>, String> {
        let mut total = 0;
        let mut messages = vec![];
        for message in &chat.messages {
            if message.content.is_empty() && message.attachments.is_empty() {
                continue;
            }
            if let Some(history) = message.extra.get("agentHistory").and_then(Value::as_array) {
                messages.extend(history.iter().cloned());
                if message
                    .extra
                    .get("agentStatus")
                    .is_some_and(|s| s == "failed" || s == "cancelled")
                {
                    messages.push(json!({"role":"assistant","content":message.content}));
                }
                continue;
            }
            let mut wire = json!({"role":message.role,"content":message.content});
            if !message.attachments.is_empty() {
                let mut images = vec![];
                for a in &message.attachments {
                    let path = self.image_path(&a.id).map_err(|e| e.to_string())?;
                    let size=fs::metadata(&path).map_err(|_|"A saved image is missing. Restore it from a backup, or start a new chat.")?.len();
                    total += size;
                    if size > 8 * 1024 * 1024 || total > 24 * 1024 * 1024 {
                        return Err("This conversation exceeds the 24 MB request image limit. Start a new chat.".into());
                    }
                    images.push(STANDARD.encode(fs::read(path).map_err(|e| e.to_string())?));
                }
                wire["images"] = json!(images);
            }
            messages.push(wire);
        }
        Ok(crate::context::complete_groups(messages))
    }
}
