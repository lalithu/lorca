pub mod authority;
pub mod context;
mod execution;
pub mod objective;
mod runtime;
pub use crate::agent_types as types;
pub mod verification;
pub use crate::tools::{Prepared, Registry, Tool, Workspace};
pub use runtime::Agent;
pub use types::*;

/// One task-scoped contract, independent of model-authored messages.
#[derive(Debug, serde::Serialize)]
pub struct TaskContract {
    pub objective: objective::Objective,
    pub context: context::TaskContext,
    pub authority: authority::Authority,
}
impl TaskContract {
    pub fn new(instruction: &str, workspace: Option<String>, conversation_messages: usize) -> Self {
        Self {
            objective: objective::Objective::new(instruction),
            context: context::TaskContext {
                workspace,
                conversation_messages,
            },
            authority: authority::Authority::new(instruction),
        }
    }
}
