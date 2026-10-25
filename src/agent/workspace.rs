use super::types::Result;
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};

pub const FILE_LIMIT: usize = 128 * 1024;
pub struct Workspace {
    pub root: PathBuf,
    dir: Dir,
    verified: std::sync::Mutex<std::collections::BTreeSet<String>>,
}
impl Workspace {
    pub fn new(path: &Path) -> Result<Self> {
        let root = path.canonicalize().map_err(|e| format!("Workspace: {e}"))?;
        if !root.is_dir() || root.parent().is_none() {
            return Err("Select a workspace directory, not the filesystem root".into());
        }
        let dir = Dir::open_ambient_dir(&root, ambient_authority()).map_err(|e| e.to_string())?;
        let workspace = Self {
            root,
            dir,
            verified: Default::default(),
        };
        // A small real inventory supplies useful paths without another inference call.
        let mut queue = std::collections::VecDeque::from([(String::from("."), 0)]);
        let started = std::time::Instant::now();
        let mut scanned = 0;
        while let Some((path, depth)) = queue.pop_front() {
            let Ok(dir) = workspace.directory(&path) else {
                continue;
            };
            let Ok(entries) = dir.entries() else { continue };
            let mut entries = entries.take(200).flatten().collect::<Vec<_>>();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                scanned += 1;
                if scanned > 200 || started.elapsed() > std::time::Duration::from_millis(100) {
                    return Ok(workspace);
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.')
                    || ["target", "node_modules", "build", "vendor"].contains(&name.as_str())
                {
                    continue;
                }
                let Ok(ty) = entry.file_type() else { continue };
                if ty.is_symlink() {
                    continue;
                }
                let reference = if path == "." {
                    name
                } else {
                    format!("{path}/{name}")
                };
                if workspace.remember(&reference).is_ok() && ty.is_dir() && depth < 2 {
                    queue.push_back((reference, depth + 1));
                }
            }
        }
        Ok(workspace)
    }
    /// Rehydrate only references backed by saved discovery/read results. Every
    /// candidate is resolved again inside the current capability before reuse.
    pub fn restore_references(&self, history: &[serde_json::Value]) {
        for message in history.iter().filter(|m| m["role"] == "tool") {
            let Ok(result) = serde_json::from_str::<serde_json::Value>(
                message["content"].as_str().unwrap_or(""),
            ) else {
                continue;
            };
            if result["ok"] != true {
                continue;
            }
            if let Some(path) = result["path"].as_str() {
                let _ = self.remember(path);
            }
            for key in ["entries", "matches"] {
                if let Some(entries) = result[key].as_array() {
                    for entry in entries {
                        if let Some(path) = entry["path"].as_str() {
                            let _ = self.remember(path);
                        }
                    }
                }
            }
        }
    }
    pub fn references(&self) -> Vec<String> {
        self.verified.lock().unwrap().iter().cloned().collect()
    }
    pub fn remember(&self, path: &str) -> Result<String> {
        let path = Self::path(path)?;
        let canonical = self
            .dir
            .canonicalize(path)
            .map_err(|e| format!("Discovery: {e}"))?;
        let relative = canonical.to_string_lossy().to_string();
        Self::path(&relative)?;
        let mut verified = self.verified.lock().unwrap();
        if verified.len() < 1000 {
            verified.insert(relative.clone());
        }
        Ok(relative)
    }
    pub fn require_reference(&self, path: &str) -> Result<()> {
        let normalized = Self::path(path)?
            .components()
            .filter_map(|c| match c {
                Component::Normal(v) => Some(v.to_string_lossy().to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/");
        if !self.verified.lock().unwrap().contains(&normalized) {
            return Err(format!(
                "Unverified file reference: {path}. Relevant verified paths: {}. Discover the actual location with list_directory or search_files and reuse its returned path.",
                self.references()
                    .into_iter()
                    .filter(|p| p.rsplit('/').next() == normalized.rsplit('/').next()
                        || p.starts_with(normalized.split('/').next().unwrap_or("")))
                    .take(8)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        // Revalidate on every use; cached discovery never replaces filesystem checks.
        let current = self
            .dir
            .canonicalize(Self::path(path)?)
            .map_err(|e| format!("File reference changed: {e}. Discover again."))?;
        if current.to_string_lossy() != normalized {
            return Err("File reference changed; discover again".into());
        }
        Ok(())
    }
    pub fn path(path: &str) -> Result<&Path> {
        let p = Path::new(path);
        if path.is_empty()
            || path.len() > 2048
            || path.chars().any(char::is_control)
            || p.components()
                .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        {
            return Err("Use a relative workspace path without traversal".into());
        }
        if p.components().any(|c| c.as_os_str() == ".git") {
            return Err("Git metadata is protected".into());
        }
        Ok(p)
    }
    pub fn directory(&self, path: &str) -> Result<Dir> {
        self.dir
            .open_dir(Self::path(path)?)
            .map_err(|e| format!("Directory {path}: {e}"))
    }
    pub fn read(&self, path: &str) -> Result<String> {
        let f = self
            .dir
            .open(Self::path(path)?)
            .map_err(|e| format!("Read {path}: {e}"))?;
        if !f.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("Expected a regular file".into());
        }
        let mut bytes = vec![];
        f.take(FILE_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > FILE_LIMIT {
            return Err("File exceeds 128 KiB; use search_files or a smaller source file".into());
        }
        String::from_utf8(bytes).map_err(|_| "File is not UTF-8 text".into())
    }
    pub fn excerpt(&self, path: &str, offset: usize, length: usize) -> Result<serde_json::Value> {
        let mut file = self
            .dir
            .open(Self::path(path)?)
            .map_err(|e| format!("Read {path}: {e}"))?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() {
            return Err("Expected a regular file".into());
        }
        let total = metadata.len();
        if offset as u64 > total {
            return Err("Read offset exceeds file length; use nextOffset".into());
        }
        file.seek(SeekFrom::Start(offset as u64))
            .map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        file.take(length as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let content = match std::str::from_utf8(&bytes) {
            Ok(text) => text,
            Err(error)
                if error.error_len().is_none() && offset as u64 + (bytes.len() as u64) < total =>
            {
                std::str::from_utf8(&bytes[..error.valid_up_to()])
                    .map_err(|_| "File is not UTF-8 text")?
            }
            Err(_) => {
                return Err(
                    "Read requires UTF-8 text and a valid byte boundary; use nextOffset".into(),
                );
            }
        };
        let end = offset as u64 + content.len() as u64;
        Ok(
            serde_json::json!({"ok":true,"path":path,"offset":offset,"content":content,
            "truncated":end<total,"nextOffset":if end<total {Some(end)} else {None},"totalBytes":total}),
        )
    }
    /// Hold the parent directory capability during validation and replacement. Reject write
    /// symlinks (including parents); an approval cannot be redirected by retargeting a link.
    fn parent(&self, path: &str) -> Result<(Dir, String)> {
        let path = Self::path(path)?;
        let mut dir = self.dir.try_clone().map_err(|e| e.to_string())?;
        let components: Vec<_> = path
            .components()
            .filter_map(|c| {
                if let Component::Normal(v) = c {
                    Some(v)
                } else {
                    None
                }
            })
            .collect();
        let (name, parents) = components.split_last().ok_or("Expected a file path")?;
        for p in parents {
            if dir
                .symlink_metadata(p)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
            {
                return Err("Patch paths cannot contain symlinks".into());
            }
            dir = dir.open_dir(p).map_err(|e| e.to_string())?;
        }
        Ok((dir, name.to_string_lossy().into()))
    }
    fn existing(dir: &Dir, name: &str) -> Result<Option<String>> {
        match dir.symlink_metadata(name) {
            Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
                Err("Patch target must be a regular file, not a symlink".into())
            }
            Ok(_) => {
                let mut s = String::new();
                dir.open(name)
                    .map_err(|e| e.to_string())?
                    .take(FILE_LIMIT as u64 + 1)
                    .read_to_string(&mut s)
                    .map_err(|e| e.to_string())?;
                if s.len() > FILE_LIMIT {
                    return Err("Patch target too large".into());
                }
                Ok(Some(s))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    pub fn patch_source(&self, path: &str) -> Result<Option<String>> {
        let (parent, name) = self.parent(path)?;
        Self::existing(&parent, &name)
    }
    pub fn replace(&self, path: &str, expected: &Option<String>, text: &str) -> Result<()> {
        let (parent, name) = self.parent(path)?;
        if Self::existing(&parent, &name)? != *expected {
            return Err("File changed since proposal; request a fresh patch and approval".into());
        }
        let tmp = format!(".lorca-{}.tmp", crate::types::id());
        let operation = || -> Result<()> {
            let mut opts = OpenOptions::new();
            opts.write(true).create_new(true);
            let mut file = parent.open_with(&tmp, &opts).map_err(|e| e.to_string())?;
            if let Ok(old) = parent.metadata(&name) {
                file.set_permissions(old.permissions())
                    .map_err(|e| e.to_string())?;
            }
            file.write_all(text.as_bytes())
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
            // Check again after preparing the replacement, before the single atomic rename.
            if Self::existing(&parent, &name)? != *expected {
                return Err("File changed while preparing patch".into());
            }
            parent
                .rename(&tmp, &parent, &name)
                .map_err(|e| e.to_string())?;
            if Self::existing(&parent, &name)?.as_deref() != Some(text) {
                return Err("Patch verification failed".into());
            }
            Ok(())
        };
        let result = operation();
        let _ = parent.remove_file(&tmp);
        result
    }
}
