//! Explicit grants only. User prose and model output never alter permissions.
use crate::{inference::Result, tools::Prepared};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug)]
pub enum Access {
    None,
    Read(PathBuf),
    Write,
    Command,
    Launch,
    Network(String),
    Grant,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    #[serde(default)]
    pub read_roots: Vec<PathBuf>,
    #[serde(default)]
    pub network_hosts: Vec<String>,
    #[serde(default)]
    pub read_only: bool,
}
impl Permissions {
    pub fn select_directory(&mut self, p: &Path) {
        if !self.read_roots.iter().any(|r| r == p) {
            self.read_roots.push(p.into());
        }
    }
    pub fn needs_approval(&self, p: &Prepared) -> Result<bool> {
        if self.read_only && matches!(p.access, Access::Write | Access::Command | Access::Launch) {
            return Err("Read-only mode blocks writes, commands and application launches. Change the explicit permission setting to continue.".into());
        }
        Ok(match &p.access {
            Access::None => false,
            Access::Read(path) => !self.read_roots.iter().any(|root| path.starts_with(root)),
            Access::Network(host) => !self.network_hosts.contains(host),
            _ => true,
        })
    }
    pub fn grant(&mut self, v: &serde_json::Value) -> Result<()> {
        let target = v["target"].as_str().ok_or("Missing grant target")?;
        match v["access"].as_str() {
            Some("read") => self.select_directory(Path::new(target)),
            Some("network") => {
                if !self.network_hosts.contains(&target.to_owned()) {
                    self.network_hosts.push(target.into());
                }
            }
            _ => return Err("Unknown permission".into()),
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Operation;
    use serde_json::json;
    #[test]
    fn paths_are_scoped_and_readonly_is_explicit() {
        let mut p = Permissions::default();
        p.select_directory(Path::new("/project"));
        let action = |a| Prepared {
            action: json!({}),
            access: a,
            operation: Operation::List {
                path: PathBuf::from("/project"),
            },
        };
        assert!(
            !p.needs_approval(&action(Access::Read("/project/a".into())))
                .unwrap()
        );
        assert!(
            p.needs_approval(&action(Access::Read("/project-other".into())))
                .unwrap()
        );
        p.read_only = true;
        assert!(p.needs_approval(&action(Access::Command)).is_err());
    }
}
