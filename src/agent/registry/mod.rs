//! Agent Registry - manages all agent lifecycles
//!
//! Provides centralized management for creating, tracking, and removing agents.

pub mod cascade;
pub mod lifecycle;
pub mod query;

use crate::agent::process::{AgentProcess, AgentProcessHandle};
#[cfg(test)]
use crate::agent::state::{is_valid_transition, Checkpoint, DestroyConfirmation, SourceLocation};
use crate::agent::state::{AgentStateTransition, ErrorInfo, SuspendedReason, TransitionTrigger};
use crate::agent::{Agent, AgentState};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

/// Errors that can occur in the agent registry
#[derive(Error, Debug)]
pub enum RegistryError {
    #[error("agent not found: {0}")]
    AgentNotFound(String),
    #[error("agent already exists: {0}")]
    AgentAlreadyExists(String),
    #[error("invalid state transition: {0}")]
    InvalidStateTransition(String),
    #[error("destroy requires confirmation (token mismatch or missing)")]
    DestroyConfirmationRequired,
    #[error("process error: {0}")]
    ProcessError(#[from] crate::agent::process::ProcessError),
}

/// Result type for registry operations
pub type RegistryResult<T> = Result<T, RegistryError>;

/// Result of a cleanup operation
#[derive(Debug)]
pub struct CleanupResult {
    /// Agents that were successfully cleaned up
    pub cleaned: Vec<String>,
    /// Agent IDs that failed to be removed, with their errors
    pub failed: Vec<(String, RegistryError)>,
}

/// Thread-safe agent registry for managing all agent lifecycles
#[derive(Debug)]
pub struct AgentRegistry {
    /// Map of agent ID to agent metadata
    pub(super) agents: RwLock<HashMap<String, Agent>>,
    /// Map of agent ID to running process handle
    pub(super) processes: RwLock<HashMap<String, AgentProcessHandle>>,
    /// Heartbeat timeout in seconds
    pub(super) heartbeat_timeout_secs: i64,
    /// Graceful shutdown: wait timeout in seconds
    pub(super) wait_timeout_secs: u64,
    /// Graceful shutdown: grace period in seconds
    pub(super) grace_period_secs: u64,
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new_with_graceful_shutdown(30, 30, 10)
    }
}

impl AgentRegistry {
    /// Create a new registry with specified heartbeat timeout.
    pub fn new(heartbeat_timeout_secs: i64) -> Self {
        Self::new_with_graceful_shutdown(heartbeat_timeout_secs, 30, 10)
    }

    /// Create a new registry with graceful shutdown configuration.
    pub fn new_with_graceful_shutdown(
        heartbeat_timeout_secs: i64,
        wait_timeout_secs: u64,
        grace_period_secs: u64,
    ) -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
            processes: RwLock::new(HashMap::new()),
            heartbeat_timeout_secs,
            wait_timeout_secs,
            grace_period_secs,
        }
    }
}

/// Wrap AgentRegistry in Arc for shared access
pub type SharedAgentRegistry = Arc<AgentRegistry>;

/// Create a new shared agent registry
pub fn create_registry(heartbeat_timeout_secs: i64) -> SharedAgentRegistry {
    Arc::new(AgentRegistry::new(heartbeat_timeout_secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_transition_validation() {
        use AgentState::*;
        assert!(is_valid_transition(&Idle, &Running));
        assert!(is_valid_transition(&Running, &Waiting));
        assert!(is_valid_transition(&Running, &Stopped));
        assert!(is_valid_transition(
            &Running,
            &Suspended(SuspendedReason::Forced)
        ));
        assert!(is_valid_transition(
            &Waiting,
            &Suspended(SuspendedReason::SelfRequested)
        ));
        assert!(is_valid_transition(
            &Suspended(SuspendedReason::Forced),
            &Running
        ));
        assert!(is_valid_transition(
            &Suspended(SuspendedReason::SelfRequested),
            &Running
        ));
        assert!(is_valid_transition(
            &Suspended(SuspendedReason::SelfRequested),
            &Stopped
        ));
        assert!(!is_valid_transition(&Stopped, &Running));
        assert!(!is_valid_transition(
            &Error(ErrorInfo::new("fatal", false)),
            &Running
        ));
        assert!(is_valid_transition(
            &Error(ErrorInfo::new("recoverable", true)),
            &Running
        ));
        assert!(is_valid_transition(&Running, &Running));
    }

    #[tokio::test]
    async fn test_graceful_shutdown_config() {
        let registry = AgentRegistry::new_with_graceful_shutdown(30, 60, 20);
        assert_eq!(registry.wait_timeout_secs(), 60);
        assert_eq!(registry.grace_period_secs(), 20);
    }
}
