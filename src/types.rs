use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub fn id() -> String {
    uuid::Uuid::new_v4().to_string().to_uppercase()
}
pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        - 978_307_200.0
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
impl Message {
    pub fn new(role: &str, content: String) -> Self {
        Self {
            id: id(),
            role: role.into(),
            content,
            attachments: vec![],
            extra: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub version: u32,
    pub id: String,
    pub title: String,
    pub created_at: f64,
    pub updated_at: f64,
    pub messages: Vec<Message>,
    pub draft: String,
    pub draft_images: Vec<Attachment>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
impl Default for Conversation {
    fn default() -> Self {
        Self {
            version: 1,
            id: id(),
            title: "New chat".into(),
            created_at: now(),
            updated_at: now(),
            messages: vec![],
            draft: String::new(),
            draft_images: vec![],
            extra: BTreeMap::new(),
        }
    }
}
impl Conversation {
    pub fn images(&self) -> impl Iterator<Item = &Attachment> {
        self.messages
            .iter()
            .flat_map(|m| m.attachments.iter())
            .chain(self.draft_images.iter())
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub context_used: Option<u64>,
    pub context_limit: Option<u64>,
    pub tokens_per_second: Option<f64>,
    #[serde(default)]
    pub prompt_tokens: Option<u64>,
    #[serde(default)]
    pub generated_tokens: Option<u64>,
    #[serde(default)]
    pub generation_duration_ns: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub id: String,
    pub selected_id: String,
    pub model: String,
    pub loading_model: Option<String>,
    pub active_model: Option<String>,
    pub models_to_release: Vec<String>,
    pub models: Vec<String>,
    pub is_connected: bool,
    pub error: Option<String>,
    pub usage: Usage,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnView {
    pub id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub model: String,
    pub error: Option<String>,
    pub running: bool,
    pub usage: Usage,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub revision: u64,
    pub conversations: Vec<Conversation>,
    pub sessions: Vec<SessionView>,
    pub turns: Vec<TurnView>,
    pub warning: Option<String>,
}
