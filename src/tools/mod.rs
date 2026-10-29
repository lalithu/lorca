//! Native tools. Registry visibility never grants execution authority.
mod agent;
pub(crate) use agent::WorkingCheckpoint;
mod arguments;
mod files;
pub mod registry;
mod terminal;
mod vision;
mod web;
mod workspace;
use crate::agent_types::*;
pub use registry::{Authority, Category, Registry};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::watch;
pub use workspace::Workspace;
pub struct Prepared {
    pub action: Value,
    pub approval: bool,
    pub payload: Value,
}
pub trait Tool: Send + Sync {
    fn schema(&self) -> Value;
    fn prepare_optional(
        &self,
        w: Option<&Workspace>,
        arguments: Value,
        limits: &Limits,
    ) -> Result<Prepared> {
        self.prepare(
            w.ok_or("Select a workspace before using file or command tools")?,
            arguments,
            limits,
        )
    }
    fn execute_optional<'a>(
        &'a self,
        w: Option<&'a Workspace>,
        p: &'a Prepared,
        l: &'a Limits,
        c: watch::Receiver<bool>,
        o: &'a mut (dyn FnMut(Value) + Send),
    ) -> FutureResult<'a, Value> {
        Box::pin(async move {
            self.execute(
                w.ok_or("Select a workspace before using file or command tools")?,
                p,
                l,
                c,
                o,
            )
            .await
        })
    }
    fn prepare(&self, workspace: &Workspace, arguments: Value, limits: &Limits)
    -> Result<Prepared>;
    fn execute<'a>(
        &'a self,
        workspace: &'a Workspace,
        prepared: &'a Prepared,
        limits: &'a Limits,
        cancel: watch::Receiver<bool>,
        output: &'a mut (dyn FnMut(Value) + Send),
    ) -> FutureResult<'a, Value>;
}

pub(super) fn schema(
    name: &str,
    description: &str,
    properties: Value,
    required: Vec<&str>,
) -> Value {
    json!({"type":"function","function":{"name":name,"description":description,"parameters":{"type":"object","properties":properties,"required":required,"additionalProperties":false}}})
}
pub(super) fn check_size(args: &Value) -> Result<()> {
    if args.to_string().len() > 64 * 1024 {
        Err("Tool arguments exceed 64 KiB".into())
    } else {
        Ok(())
    }
}
pub(super) fn prepared(args: Value, approval: bool) -> Result<Prepared> {
    check_size(&args)?;
    Ok(Prepared {
        action: args.clone(),
        payload: args,
        approval,
    })
}
