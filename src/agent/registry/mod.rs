//! Agent Registry - manages all agent lifecycles
//!
//! Provides centralized management for creating, tracking, and removing agents.

pub mod lifecycle;
pub mod query;

use crate::agent::process::{AgentProcess, AgentProcessHandle};
use crate::agent::Agent;
use crate::config::agents::ResolvedAgentConfig;
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

/// Thread-safe agent registry for managing all agent lifecycles
#[derive(Debug)]
pub struct AgentRegistry {
    /// Map of agent ID to agent metadata
    pub(super) agents: RwLock<HashMap<String, Agent>>,
    /// Map of agent ID to running process handle
    pub(super) processes: RwLock<HashMap<String, AgentProcessHandle>>,
    /// Map of agent ID to resolved agent config (read-only query layer)
    pub(super) configs: RwLock<HashMap<String, ResolvedAgentConfig>>,
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new_with_graceful_shutdown(30)
    }
}

impl AgentRegistry {
    /// Create a new registry. The `heartbeat_timeout_secs` parameter is
    /// retained for backward compatibility (20+ existing call sites) but
    /// is no longer used; runtime heartbeat/liveness tracking is owned by
    /// the session module.
    pub fn new(_heartbeat_timeout_secs: i64) -> Self {
        Self::new_with_graceful_shutdown(30)
    }

    /// Create a new registry with graceful shutdown configuration.
    /// `heartbeat_timeout_secs` is retained for backward compatibility but
    /// is no longer used; runtime heartbeat/liveness tracking is owned by
    /// the session module.
    pub fn new_with_graceful_shutdown(_heartbeat_timeout_secs: i64) -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
            processes: RwLock::new(HashMap::new()),
            configs: RwLock::new(HashMap::new()),
        }
    }

    /// Populate the config store with the given resolved agent configs.
    /// Called once at startup by Daemon after ConfigManager loads agents.
    pub async fn populate(&self, configs: Vec<ResolvedAgentConfig>) {
        let mut map = self.configs.write().await;
        for cfg in configs {
            map.insert(cfg.id.clone(), cfg);
        }
    }

    /// Look up a resolved agent config by ID.
    /// Returns `None` if no config with the given ID exists.
    pub async fn get_config(&self, id: &str) -> Option<ResolvedAgentConfig> {
        let map = self.configs.read().await;
        map.get(id).cloned()
    }

    /// Replace all stored configs with the given set (hot-reload).
    pub async fn reload_config(&self, configs: Vec<ResolvedAgentConfig>) {
        let mut map = self.configs.write().await;
        map.clear();
        for cfg in configs {
            map.insert(cfg.id.clone(), cfg);
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
    // Note: state-machine tests (`test_state_transition_validation` /
    // `test_graceful_shutdown_config`) were removed in Step 1.3 along
    // with the registry's `update_state` API and the `wait_timeout_secs`
    // / `grace_period_secs` knobs. The `AgentState` machine itself still
    // lives in `crate::agent::state` and is exercised by its own unit
    // tests.
}
