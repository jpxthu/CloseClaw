//! Spawn adapter trait — abstracts sub-agent spawning for testability.

use async_trait::async_trait;

use crate::error::ExecutionError;
use crate::types::SubAgentResult;

/// Errors specific to the spawn adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnError {
    /// Descriptive error message.
    pub message: String,
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "spawn error: {}", self.message)
    }
}

impl std::error::Error for SpawnError {}

/// Adapter trait for spawning sub-agents.
///
/// Implementations handle the actual spawn mechanism (process, thread,
/// or in-memory mock). The execution engine depends only on this trait,
/// keeping spawn details decoupled from scheduling logic.
#[async_trait]
pub trait SpawnAdapter: Send + Sync {
    /// Spawn a sub-agent to run a single step.
    ///
    /// Returns a structured [`SubAgentResult`] on completion.
    async fn spawn_run(&self, task: &str, context: &str) -> Result<SubAgentResult, ExecutionError>;

    /// Spawn a sub-agent session that runs a full plan.
    ///
    /// Returns the session identifier on success.
    async fn spawn_session(&self, task: &str, context: &str) -> Result<String, ExecutionError>;
}
