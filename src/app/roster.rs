//! Persistent work list. Execution stays in the existing chat runtime.
use crate::{
    storage::atomic,
    types::{Conversation, id, now},
};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub objective: String,
    pub status: String,
    pub conversation_id: Option<String>,
    pub run_id: Option<String>,
    pub model: Option<String>,
    pub summary: Option<String>,
    pub updated_at: f64,
}
impl Task {
    pub fn new(objective: String) -> Self {
        Self {
            id: id(),
            objective,
            status: "queued".into(),
            conversation_id: None,
            run_id: None,
            model: None,
            summary: None,
            updated_at: now(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Roster {
    pub tasks: Vec<Task>,
    #[serde(skip_deserializing)]
    pub error: Option<String>,
}
impl Roster {
    pub fn load(root: &Path, chats: &[Conversation]) -> Self {
        let mut roster: Self = match fs::read(root.join("roster.json")) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(value) => value,
                Err(e) => {
                    return Self {
                        error: Some(format!(
                            "Could not read roster; original file preserved: {e}"
                        )),
                        ..Self::default()
                    };
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                return Self {
                    error: Some(format!("Could not read roster: {e}")),
                    ..Self::default()
                };
            }
        };
        for task in &mut roster.tasks {
            if let Some(cid) = &task.conversation_id {
                if let Some(chat) = chats.iter().find(|c| &c.id == cid) {
                    Self::observe(task, chat);
                }
                // Restart never schedules or replays an interrupted action.
                if matches!(
                    task.status.as_str(),
                    "running" | "awaiting_approval" | "queued"
                ) {
                    task.status = "incomplete".into();
                }
            }
        }
        roster
    }
    pub fn save(&self, root: &Path) -> Result<(), String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        atomic(
            &root.join("roster.json"),
            &serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("Could not save roster: {e}"))
    }
    fn observe(task: &mut Task, chat: &Conversation) -> bool {
        let Some(message) = chat
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && m.extra.contains_key("agentStatus"))
        else {
            return false;
        };
        let status = message.extra["agentStatus"]
            .as_str()
            .unwrap_or("incomplete");
        let run = message
            .extra
            .get("agentTaskId")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        let summary = if message.content.is_empty() {
            None
        } else {
            Some(message.content.chars().take(240).collect())
        };
        if task.status == status && task.run_id == run && task.summary == summary {
            return false;
        }
        task.status = status.into();
        task.run_id = run;
        task.model = message
            .extra
            .get("agentModel")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .or_else(|| task.model.clone());
        task.summary = summary;
        task.updated_at = now();
        true
    }
    pub fn observe_chat(&mut self, chat: &Conversation) -> bool {
        let mut changed = false;
        for task in &mut self.tasks {
            if task.conversation_id.as_deref() == Some(&chat.id) {
                changed |= Self::observe(task, chat);
            }
        }
        changed
    }
}
