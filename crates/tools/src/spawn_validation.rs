//! Spawn validation trait for decoupling tools from the agent spawn module.
//!
//! Provides an interface for validating spawn requests without requiring
//! a direct dependency on the concrete `SpawnController`.

use async_trait::async_trait;

/// Errors returned by spawn validation.
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("spawn depth limit exceeded: current depth {current} >= max {max}")]
    DepthExceeded { current: u32, max: u32 },
    #[error("max concurrent children reached: {current} >= {max}")]
    MaxChildrenReached { current: usize, max: u32 },
    #[error("agent '{agent_id}' not in allowlist")]
    AgentNotAllowed { agent_id: String },
    #[error("agentId is required by configuration")]
    AgentIdRequired,
    #[error("agent config not found: {0}")]
    ConfigNotFound(String),
    #[error("spawn permission denied for agent '{agent_id}': {reason}")]
    PermissionDenied { agent_id: String, reason: String },
}

/// Result of a successful spawn validation.
///
/// Contains the resolved target agent configuration and the effective
/// max spawn depth for the child.
#[derive(Debug, Clone)]
pub struct SpawnValidationResult {
    /// Resolved configuration of the target agent.
    pub config: closeclaw_config::agents::ResolvedAgentConfig,
    /// Effective max spawn depth the child may use.
    pub effective_max_spawn_depth: u32,
}

/// Trait for validating spawn requests.
///
/// Implemented by `SpawnController` in the main crate; used by the tools
/// crate's `SessionsSpawnTool` to validate spawn requests.
#[async_trait]
pub trait SpawnValidator: Send + Sync {
    /// Validate a spawn request for the given parent session.
    ///
    /// Returns a [`SpawnValidationResult`] with the resolved target info,
    /// or a [`SpawnError`] on failure.
    async fn validate_spawn(
        &self,
        parent_session_id: &str,
        target_agent_id: Option<&str>,
    ) -> Result<SpawnValidationResult, SpawnError>;
}
