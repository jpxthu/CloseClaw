//! Agent Registry - stores resolved agent configurations.
//!
//! This module is a pure configuration layer: it holds the resolved configs
//! populated once at startup (or reloaded at runtime) and exposes read-only
//! queries. All runtime state (processes, lifecycle) lives elsewhere.

use crate::config::agents::ResolvedAgentConfig;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Thread-safe agent configuration registry.
#[derive(Debug)]
pub struct AgentRegistry {
    /// Map of agent ID to resolved agent config.
    pub(super) configs: RwLock<HashMap<String, ResolvedAgentConfig>>,
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new_with_graceful_shutdown(30)
    }
}

impl AgentRegistry {
    /// Create a new registry.
    pub fn new(_heartbeat_timeout_secs: i64) -> Self {
        Self::new_with_graceful_shutdown(30)
    }

    /// Create a new registry with graceful shutdown configuration.
    pub fn new_with_graceful_shutdown(_heartbeat_timeout_secs: i64) -> Self {
        Self {
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
    pub async fn get(&self, id: &str) -> Option<ResolvedAgentConfig> {
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
mod config_tests;
