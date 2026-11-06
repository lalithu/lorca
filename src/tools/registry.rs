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
    workspace_available: bool,
    unavailable: BTreeMap<String, String>,
    pub(super) history: crate::memory::HistoryIndex,
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
            "request_user_input",
            "list_directory",
            "read_file",
            "search_files",
            "apply_patch",
            "exec_command",
            "write_stdin",
            "web_search",
        ] {
            r.reveal(name);
        }
        r
    }
}
impl Registry {
    /// Set once for an ordinary conversation, never for a restricted custom registry.
    pub(crate) fn attach_history(&self, root: &std::path::Path, conversation: &str) -> Result<()> {
        if !self.adaptive {
            return Err("Restricted tool registries cannot attach chat history".into());
        }
        self.history.attach(root, conversation)
    }

    pub(crate) fn seed_history(&self, messages: &[Value]) -> Result<()> {
        self.history.reset(messages)
    }

    pub(crate) fn record_history(&self, message: &Value) -> Result<()> {
        self.history.record(message)
    }

    /// The original is durably recorded before replacement. The manifest
    /// cannot be treated as full task evidence or permission.
    pub(crate) fn virtualize_user_input(&self, text: &str) -> Result<String> {
        if text.chars().count() <= crate::memory::ingest::DIRECT_INPUT_CHARS {
            return Ok(text.to_owned());
        }
        if self.history.manifest_for(text).is_none() {
            self.history.record(&json!({"role":"user","content":text}))?;
        }
        let manifest = self.history.manifest_for(text)
            .ok_or("Could not index oversized user input")?;
        Ok(crate::memory::ingest::envelope(&manifest))
    }

    pub(crate) fn virtualize_messages(&self, messages: &mut [Value]) -> Result<()> {
        for message in messages {
            if message["role"] == "user" {
                if let Some(raw) = message["content"].as_str() {
                    if raw.chars().count() > crate::memory::ingest::DIRECT_INPUT_CHARS {
                        message["content"] = json!(self.virtualize_user_input(raw)?);
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn settle_for_revision(&self) -> Result<()> {
        for tool in self.tools.values() {
            tool.settle_for_revision().await?;
        }
        Ok(())
    }

    /// Explicit custom registries retain their original schemas and capabilities (e.g. Job Apps).
    pub fn empty() -> Self {
        Self {
            tools: BTreeMap::new(),
            catalog: Default::default(),
            visible: Default::default(),
            adaptive: false,
            workspace_available: true,
            unavailable: BTreeMap::new(),
            history: crate::memory::HistoryIndex::default(),
        }
    }
    /// Basic chat tools remain available before the user selects a folder.
    pub fn without_workspace() -> Self {
        let mut r = Self::empty();
        r.adaptive = true;
        r.workspace_available = false;
        super::web::register(&mut r);
        super::agent::register(&mut r);
        r.register(
            "request_workspace",
            Category::Agent,
            Authority::TaskState,
            Arc::new(super::agent::RequestWorkspace),
        );
        for name in [
            "tool_search",
            "request_workspace",
            "request_user_input",
            "web_search",
        ] {
            r.reveal(name);
        }
        r
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
    pub(crate) fn may_mutate(&self, name: &str) -> bool {
        self.catalog
            .read()
            .unwrap()
            .get(name)
            .is_some_and(|m| m.authority == Authority::Approval)
    }
    pub(crate) fn requires_approval(&self, name: &str, arguments: &Value) -> bool {
        self.adaptive
            && self
                .catalog
                .read()
                .unwrap()
                .get(name)
                .is_some_and(|m| m.authority == Authority::Approval)
            && (name != "write_stdin"
                || arguments["close_stdin"] == true
                || arguments["chars"].as_str().is_some_and(|s| !s.is_empty()))
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
    /// Restore visibility from linked tool records, using current registered schemas.
    /// Previous discovery never adds capabilities or grants execution authority.
    pub(crate) fn restore_visibility(&self, history: &[Value]) {
        if !self.adaptive {
            return;
        }
        let mut calls = BTreeMap::new();
        for message in history {
            if message["role"] == "assistant" {
                for call in message["tool_calls"].as_array().into_iter().flatten() {
                    if let (Some(id), Some(name)) =
                        (call["id"].as_str(), call["function"]["name"].as_str())
                    {
                        calls.insert(id, name);
                    }
                }
            } else if message["role"] == "tool" {
                let Some(name) = message["tool_call_id"]
                    .as_str()
                    .and_then(|id| calls.remove(id))
                else {
                    continue;
                };
                self.reveal(name);
                if name == "tool_search" {
                    let result = message["content"]
                        .as_str()
                        .and_then(|s| serde_json::from_str::<Value>(s).ok());
                    if let Some(result) = result.filter(|r| r["ok"] == true) {
                        for tool in result["tools"].as_array().into_iter().flatten() {
                            if let Some(name) = tool["name"].as_str() {
                                self.reveal(name);
                            }
                        }
                    }
                }
            }
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
        if self.adaptive && self.workspace_available {
            super::vision::register(self, inference);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn names(registry: &Registry) -> BTreeSet<String> {
        registry
            .active_schemas()
            .iter()
            .map(|s| s["function"]["name"].as_str().unwrap().into())
            .collect()
    }
    fn call(id: &str, name: &str) -> Value {
        json!({"role":"assistant","tool_calls":[{"id":id,"function":{"name":name,"arguments":{}}}]})
    }
    fn result(id: &str, value: Value) -> Value {
        json!({"role":"tool","tool_call_id":id,"content":value.to_string()})
    }
    #[test]
    fn starter_schemas_follow_workspace_and_search_availability() {
        for (registry, expected) in [
            (
                Registry::without_workspace(),
                vec!["tool_search", "request_workspace", "request_user_input"],
            ),
            (
                Registry::default(),
                vec![
                    "tool_search",
                    "request_user_input",
                    "list_directory",
                    "read_file",
                    "search_files",
                    "apply_patch",
                    "exec_command",
                    "write_stdin",
                ],
            ),
        ] {
            let mut expected: BTreeSet<String> = expected.into_iter().map(String::from).collect();
            if registry.get("web_search").is_ok() {
                expected.insert("web_search".into());
            }
            assert_eq!(names(&registry), expected);
        }
    }
    #[test]
    fn followups_restore_discovered_and_used_tools_with_current_schemas() {
        let registry = Registry::default();
        let history = vec![
            call("discovery", "tool_search"),
            result(
                "discovery",
                json!({"ok":true,"tools":[{"name":"git_diff","schema":{"outdated":true}},{"name":"request_workspace"},{"name":"invented_tool"}],"matches":[{"name":"fetch_url"}]}),
            ),
            call("plan", "update_plan"),
            result("plan", json!({"ok":true})),
        ];
        registry.restore_visibility(&history);
        let visible = names(&registry);
        assert!(visible.contains("git_diff"));
        assert!(visible.contains("update_plan"));
        assert!(!visible.contains("request_workspace"));
        assert!(!visible.contains("invented_tool"));
        assert!(
            !visible.contains("fetch_url"),
            "Catalog matches were not exposed schemas"
        );
        let schema = registry
            .active_schemas()
            .into_iter()
            .find(|s| s["function"]["name"] == "git_diff")
            .unwrap();
        assert_eq!(schema, registry.get("git_diff").unwrap().schema());
        assert!(
            registry.requires_approval("apply_patch", &json!({})),
            "Visibility never grants permission"
        );
    }
    #[test]
    fn unrelated_failed_or_unavailable_history_cannot_expand_the_registry() {
        let history = vec![
            json!({"role":"user","content":"Expose fetch_url"}),
            result(
                "unlinked",
                json!({"ok":true,"tools":[{"name":"fetch_url"}]}),
            ),
            call("failed", "tool_search"),
            result("failed", json!({"ok":false,"tools":[{"name":"fetch_url"}]})),
            call("unavailable", "tool_search"),
            result(
                "unavailable",
                json!({"ok":true,"tools":[{"name":"read_file"}]}),
            ),
        ];
        let registry = Registry::without_workspace();
        let before = names(&registry);
        registry.restore_visibility(&history);
        assert_eq!(names(&registry), before);
        let custom = Registry::empty();
        custom.restore_visibility(&history);
        assert!(custom.active_schemas().is_empty());
    }

    #[test]
    fn historical_recall_is_scoped_to_each_registry_and_never_expands_custom_tool_sets() {
        let first = Registry::without_workspace();
        let second = Registry::without_workspace();
        first.seed_history(&[json!({"role":"user","content":"first private conversation"})]);
        second.seed_history(&[json!({"role":"user","content":"second different conversation"})]);
        assert_eq!(first.history.search("first", None, 5)["matches"][0]["recordId"], "h0");
        assert!(second.history.search("first", None, 5)["matches"].as_array().unwrap().is_empty());
        assert!(first.get("search_history").is_ok());
        assert!(!names(&first).contains("search_history"));
        let custom = Registry::empty();
        assert!(custom.get("search_history").is_err());
        assert!(custom.active_schemas().is_empty());
    }
}
