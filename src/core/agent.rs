use super::*;
use crate::agent::{Agent, Decision, FutureResult, Inference, Turn};

pub(super) struct CoreInference {
    pub(super) core: Arc<Core>,
    pub(super) session: String,
    pub(super) model: String,
}
/// Keep the existing model preparation future alive until it has cleaned up its
/// loading state, even when a task timeout/cancellation drops its inference future.
struct LoadGuard(watch::Sender<bool>);
impl Drop for LoadGuard {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}
impl Inference for CoreInference {
    fn generate<'a>(&'a self, messages: &'a [Value], tools: &'a [Value]) -> FutureResult<'a, Turn> {
        Box::pin(async move {
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
            let provider = provider::agent::OllamaAgent {
                client: self.core.client.clone(),
                host: self.core.host(&self.session)?,
                model: self.model.clone(),
            };
            provider.generate(messages, tools).await
        })
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
