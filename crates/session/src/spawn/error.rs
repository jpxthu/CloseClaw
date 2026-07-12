//! Errors returned by spawn validation and child session creation.

use thiserror::Error;

/// Errors returned by SpawnController validation.
#[derive(Debug, Error)]
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
