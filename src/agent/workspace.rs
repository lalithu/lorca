use super::types::Result;
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use std::{
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

pub const FILE_LIMIT: usize = 128 * 1024;
pub struct Workspace {
    pub root: PathBuf,
    dir: Dir,
}
impl Workspace {
    pub fn new(path: &Path) -> Result<Self> {
        let root = path.canonicalize().map_err(|e| format!("Workspace: {e}"))?;
        if !root.is_dir() || root.parent().is_none() {
            return Err("Select a workspace directory, not the filesystem root".into());
        }
        let dir = Dir::open_ambient_dir(&root, ambient_authority()).map_err(|e| e.to_string())?;
        Ok(Self { root, dir })
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
