use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{future::Future, pin::Pin};

pub type Result<T> = std::result::Result<T, String>;
pub type FutureResult<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Only structured calls reach tools. Assistant prose never authorizes an action.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}
#[derive(Clone, Debug)]
pub struct Turn {
    pub content: String,
    pub calls: Vec<ToolCall>,
    pub message: Value,
}
pub trait Inference: Send + Sync {
    fn generate<'a>(&'a self, messages: &'a [Value], tools: &'a [Value]) -> FutureResult<'a, Turn>;
    fn generate_stream<'a>(
        &'a self,
        messages: &'a [Value],
        tools: &'a [Value],
        _updates: &'a (dyn Fn(ModelUpdate) + Send + Sync),
    ) -> FutureResult<'a, Turn> {
        self.generate(messages, tools)
    }
    fn capacity(&self) -> FutureResult<'_, Option<u64>> {
        Box::pin(async { Ok(None) })
    }
}
#[derive(Clone, Debug)]
pub enum ModelUpdate {
    Text(String),
    Usage(crate::types::Usage),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Limits {
    pub iterations: usize,
    pub model_seconds: u64,
    pub command_seconds: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            iterations: 20,
            model_seconds: 120,
            command_seconds: 30,
        }
    }
}
impl Limits {
    pub fn validate(&self) -> Result<()> {
        if !(1..=32).contains(&self.iterations)
            || !(1..=180).contains(&self.model_seconds)
            || !(1..=120).contains(&self.command_seconds)
        {
            return Err("Limits: iterations 1–32, modelSeconds 1–180, commandSeconds 1–120".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    AwaitingApproval,
    Completed,
    Cancelled,
    Failed,
}
impl Status {
    pub fn active(&self) -> bool {
        matches!(self, Self::Running | Self::AwaitingApproval)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    pub sequence: u64,
    pub task_id: String,
    pub kind: String,
    pub data: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub id: String,
    pub call_id: String,
    pub tool: String,
    pub action: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskView {
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
    pub id: String,
    pub owner: String,
    pub model: Option<String>,
    pub task: String,
    pub workspace: String,
    pub status: Status,
    pub events: Vec<Event>,
    pub approval: Option<Approval>,
    pub result: Option<String>,
    #[serde(default)]
    pub assistant_text: String,
    #[serde(default)]
    pub usage: crate::types::Usage,
    #[serde(default)]
    pub generating: bool,
    /// Full model/tool history is persisted separately; never included in UI snapshots.
    #[serde(skip)]
    pub transcript: Vec<Value>,
    #[serde(skip)]
    pub audit: Vec<Event>,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    AllowOnce,
    Deny,
    Cancel,
}

pub fn bounded(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.into();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[output truncated]", &text[..end])
}

pub struct Submission {
    pub owner: String,
    pub model: Option<String>,
    pub task: String,
    pub workspace: std::path::PathBuf,
    pub limits: Limits,
    pub conversation_id: Option<String>,
    pub request_id: Option<String>,
    pub history: Vec<Value>,
}
