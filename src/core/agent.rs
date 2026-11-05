use super::*;
use crate::agent::{Agent, Decision, FutureResult, Inference, ModelUpdate, Turn};

pub(super) struct CoreInference {
    pub(super) core: Arc<Core>,
    pub(super) session: String,
    pub(super) model: String,
    pub(super) engine: provider::Engine,
    pub(super) effort: crate::agent_types::ThinkingEffort,
}
/// Keep the existing model preparation future alive until it has cleaned up its
/// loading state, even when a task timeout/cancellation drops its inference future.
struct LoadGuard(watch::Sender<bool>);
impl Drop for LoadGuard {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}
impl CoreInference {
    async fn ready(&self) -> Result<()> {
        let core = self.core.clone();
        let session = self.session.clone();
        let model = self.model.clone();
        let (tx, mut cancel) = watch::channel(false);
        let _guard = LoadGuard(tx);
        let prepared=tokio::spawn(async move {
                let operation=core.prepare(&session,&model,false);tokio::pin!(operation);
                tokio::select! {biased;
                    _=cancel.changed()=>{
                        let loading={let s=core.state.lock().unwrap();s.loads.get(&session).map(|tx|tx.send_replace(true)).is_some()};
                        if loading { operation.await } else { Err("Model loading cancelled.".into()) }
                    },
                    result=&mut operation=>result,
                }
            }).await.map_err(|e|format!("Model preparation: {e}"))??;
        if let Some(error) = prepared["sessions"]
            .as_array()
            .and_then(|s| s.iter().find(|v| v["id"] == self.session))
            .and_then(|v| v["error"].as_str())
        {
            return Err(error.into());
        }
        Ok(())
    }
}
impl Inference for CoreInference {
    fn effort(&self) -> crate::agent_types::ThinkingEffort {
        self.effort
    }
    fn generation_policy(
        &self,
        budget: crate::context::GenerationBudget,
        seconds: u64,
        usage: &crate::types::Usage,
    ) -> (crate::context::GenerationBudget, u64) {
        budget.scheduled(seconds, usage, self.effort)
    }
    fn generate<'a>(&'a self, messages: &'a [Value], tools: &'a [Value]) -> FutureResult<'a, Turn> {
        self.generate_stream(messages, tools, &|_| {})
    }
    fn capacity(&self) -> FutureResult<'_, Option<u64>> {
        Box::pin(async {
            self.ready().await?;
            Ok(self
                .core
                .context_limit_for(self.engine, &self.core.host(&self.session)?, &self.model)
                .await)
        })
    }
    fn generate_stream<'a>(
        &'a self,
        messages: &'a [Value],
        tools: &'a [Value],
        updates: &'a (dyn Fn(ModelUpdate) + Send + Sync),
    ) -> FutureResult<'a, Turn> {
        Box::pin(async move {
            self.ready().await?;
            self.provider()
                .generate_stream(messages, tools, updates)
                .await
        })
    }
    fn generate_budgeted<'a>(
        &'a self,
        messages: &'a [Value],
        tools: &'a [Value],
        budget: crate::context::GenerationBudget,
        updates: &'a (dyn Fn(ModelUpdate) + Send + Sync),
    ) -> FutureResult<'a, Turn> {
        self.generate_at_stage(
            messages,
            tools,
            budget,
            updates,
            crate::agent_types::ThinkingStage::Execution,
        )
    }
}
impl CoreInference {
    fn generate_at_stage<'a>(
        &'a self,
        messages: &'a [Value],
        tools: &'a [Value],
        budget: crate::context::GenerationBudget,
        updates: &'a (dyn Fn(ModelUpdate) + Send + Sync),
        stage: crate::agent_types::ThinkingStage,
    ) -> FutureResult<'a, Turn> {
        Box::pin(async move {
            self.ready().await?;
            let provider = self.provider();
            let forward = |update| match update {
                ModelUpdate::Reasoning {
                    request_id,
                    state,
                    text,
                } => self
                    .core
                    .agent()
                    .reasoning_update(request_id, stage, state, text),
                other => updates(other),
            };
            provider
                .generate_for_effort(messages, tools, budget, self.effort, &forward)
                .await
        })
    }
}
impl CoreInference {
    fn provider(&self) -> provider::llama_cpp::LocalAgent {
        provider::llama_cpp::LocalAgent {
            client: self.core.client.clone(),
            host: self
                .core
                .host(&self.session)
                .unwrap_or_else(|_| self.engine.default_host().into()),
            model: self.model.clone(),
            engine: self.engine,
        }
    }
}

impl Core {
    pub(super) fn agent_command(
        self: &Arc<Self>,
        session: &str,
        op: &str,
        v: &Value,
    ) -> Result<Value> {
        match op {
            "agent_start" => self.start_agent(session, v),
            "agent_revise" => {
                self.agent
                    .revise(session, text(v, "taskId")?, text(v, "task")?)?;
                Ok(json!({"accepted":true,"taskId":v["taskId"]}))
            }
            "agent_status" => {
                let task = self.agent.snapshot().ok_or("No active task")?;
                if task.owner != session {
                    return Err("Task belongs to another session".into());
                }
                Ok(
                    json!({"taskId":task.id,"status":task.status,"task":task.task,"revision":self.agent.objective_revision()}),
                )
            }
            "agent_snapshot" => Ok(json!(self.agent.snapshot())),
            "agent_decide" => {
                let decision: Decision = serde_json::from_value(v["decision"].clone())
                    .map_err(|_| "Use allow_once, deny, or cancel")?;
                self.agent.decide(
                    session,
                    text(v, "taskId")?,
                    text(v, "approvalId")?,
                    decision,
                )?;
                Ok(json!({"accepted":true}))
            }
            "agent_cancel" => {
                self.agent.cancel(session, text(v, "taskId")?)?;
                Ok(json!({"accepted":true}))
            }
            _ => Err("Unknown agent operation".into()),
        }
    }
    pub fn agent(&self) -> &Arc<Agent> {
        &self.agent
    }
}
