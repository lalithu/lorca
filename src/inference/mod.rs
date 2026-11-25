pub mod providers;

use crate::types::{ModelChoice, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    #[default]
    Ollama,
    LlamaCpp,
}
impl Engine {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::LlamaCpp => "llama.cpp",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Call {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}
pub struct Turn {
    pub message: Value,
    pub calls: Vec<Call>,
    pub length: bool,
}
pub enum Update {
    Text(String),
    Reasoning(String),
    Usage(Usage),
}
#[derive(Clone)]
pub struct Model {
    pub choice: ModelChoice,
    pub context: Option<u64>,
    pub effort: String,
    pub client: reqwest::Client,
    pub properties: std::sync::Arc<std::sync::Mutex<Value>>,
}
impl Model {
    pub fn new(choice: ModelChoice, context: Option<u64>, effort: String) -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            choice,
            context,
            effort,
            client,
            properties: std::sync::Arc::new(std::sync::Mutex::new(serde_json::json!({}))),
        })
    }
    /// Count the provider-rendered prompt where supported. Ollama has no public tokenizer.
    pub async fn count_tokens(&self, messages: &[Value], tools: &[Value]) -> Option<u64> {
        if self.choice.route.provider != Engine::LlamaCpp
            || messages.iter().any(|m| m.get("images").is_some())
            || self.properties.lock().unwrap()["lorca_no_tokenizer"] == true
        {
            return None;
        }
        let result=async {
            let mut body=providers::llama_cpp::request_body(self,messages,tools,0);
            body["add_generation_prompt"]=serde_json::json!(true);
            let response=self.client.post(format!("{}/apply-template",self.choice.route.host))
                .json(&body)
                .timeout(std::time::Duration::from_secs(5)).send().await.ok()?.error_for_status().ok()?.json::<Value>().await.ok()?;
            let prompt=response["prompt"].as_str()?;
            let tokens=self.client.post(format!("{}/tokenize",self.choice.route.host))
                .json(&serde_json::json!({"model":self.choice.model,"content":prompt,"add_special":false,"parse_special":true}))
                .timeout(std::time::Duration::from_secs(5)).send().await.ok()?.error_for_status().ok()?.json::<Value>().await.ok()?;
            Some(tokens["tokens"].as_array()?.len() as u64)
        }.await;
        if result.is_none() {
            self.properties.lock().unwrap()["lorca_no_tokenizer"] = serde_json::json!(true);
        }
        result
    }
    pub async fn generate(
        &self,
        messages: &[Value],
        tools: &[Value],
        output: u64,
        updates: &(dyn Fn(Update) + Send + Sync),
    ) -> Result<Turn> {
        match self.choice.route.provider {
            Engine::Ollama => {
                crate::providers::ollama::generate(self, messages, tools, output, updates).await
            }
            Engine::LlamaCpp => {
                crate::providers::llama_cpp::generate(self, messages, tools, output, updates).await
            }
        }
    }
}
