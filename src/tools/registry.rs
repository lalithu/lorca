use super::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Mutex, RwLock},
};

#[derive(Clone, Copy, Debug, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Files,
    Terminal,
    Vision,
    Web,
    Agent,
}
#[derive(Clone, Copy, Debug, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    WorkspaceRead,
    Approval,
    PublicWeb,
    TaskState,
}
#[derive(Clone, serde::Serialize)]
pub struct Metadata {
    pub name: String,
    pub description: String,
    pub category: Category,
    pub authority: Authority,
    pub schema: Value,
}
pub(super) type Catalog = Arc<RwLock<BTreeMap<String, Metadata>>>;
pub(super) type Visible = Arc<Mutex<BTreeSet<String>>>;

pub struct Registry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
    pub(super) catalog: Catalog,
    pub(super) visible: Visible,
    adaptive: bool,
    unavailable: BTreeMap<String, String>,
}
impl Default for Registry {
    fn default() -> Self {
        let mut r = Self::empty();
        r.adaptive = true;
        super::files::register(&mut r);
        super::terminal::register(&mut r);
        super::web::register(&mut r);
        super::agent::register(&mut r);
        r.unavailable.insert(
            "view_image".into(),
            "Available when an inference session is attached".into(),
        );
        for name in [
            "tool_search",
            "list_directory",
            "read_file",
            "search_files",
            "apply_patch",
            "request_user_input",
            "exec_command",
            "write_stdin",
        ] {
            r.visible.lock().unwrap().insert(name.into());
        }
        r
    }
}
impl Registry {
    /// Explicit custom registries retain their original schemas and capabilities (e.g. Job Apps).
    pub fn empty() -> Self {
        Self {
            tools: BTreeMap::new(),
            catalog: Default::default(),
            visible: Default::default(),
            adaptive: false,
            unavailable: BTreeMap::new(),
        }
    }
    /// Compatibility API for existing independent Tool implementations.
    pub fn add(&mut self, name: &str, tool: Arc<dyn Tool>) {
        self.register(name, Category::Agent, Authority::Approval, tool);
    }
    pub fn register(
        &mut self,
        name: &str,
        category: Category,
        authority: Authority,
        tool: Arc<dyn Tool>,
    ) {
        let schema = tool.schema();
        self.catalog.write().unwrap().insert(
            name.into(),
            Metadata {
                name: name.into(),
                description: schema["function"]["description"]
                    .as_str()
                    .unwrap_or("")
                    .into(),
                category,
                authority,
                schema,
            },
        );
        self.unavailable.remove(name);
        self.tools.insert(name.into(), tool);
    }
    pub fn get(&self, name: &str) -> Result<Arc<dyn Tool>> {
        self.tools.get(name).cloned().ok_or_else(|| {
            self.unavailable
                .get(name)
                .map(|reason| format!("{name} unavailable: {reason}"))
                .unwrap_or_else(|| format!("Unknown tool: {name}"))
        })
    }
    pub fn schemas(&self) -> Vec<Value> {
        self.tools.values().map(|t| t.schema()).collect()
    }
    /// Present a registered tool's schema on the next turn after a format error.
    /// Visibility never grants authority or adds a tool to a restricted registry.
    pub(crate) fn reveal(&self, name: &str) {
        if self.adaptive && self.tools.contains_key(name) {
            self.visible.lock().unwrap().insert(name.into());
        }
    }
    pub fn active_schemas(&self) -> Vec<Value> {
        if !self.adaptive {
            return self.schemas();
        }
        let visible = self.visible.lock().unwrap();
        self.tools
            .iter()
            .filter(|(name, _)| visible.contains(*name))
            .map(|(_, tool)| tool.schema())
            .collect()
    }
    pub fn metadata(&self) -> Vec<Metadata> {
        self.catalog.read().unwrap().values().cloned().collect()
    }
    pub fn unavailable(&self) -> &BTreeMap<String, String> {
        &self.unavailable
    }
    pub(super) fn mark_unavailable(&mut self, name: &str, reason: &str) {
        self.unavailable.insert(name.into(), reason.into());
    }
    pub(crate) fn attach_inference(&mut self, inference: Arc<dyn Inference>) {
        // Never expand an intentionally restricted/custom registry.
        if self.adaptive {
            super::vision::register(self, inference);
        }
    }
}
