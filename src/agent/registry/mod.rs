//! Agent Registry - stores resolved agent configurations.
//!
//! This module is a pure configuration layer: it holds the resolved configs
//! populated once at startup (or reloaded at runtime) and exposes read-only
//! queries. All runtime state (processes, lifecycle) lives elsewhere.

use crate::config::agents::ResolvedAgentConfig;
use crate::session::bootstrap::loader::BootstrapMode;
use dashmap::DashMap;
use std::sync::Arc;

/// Thread-safe agent configuration registry.
#[derive(Debug)]
pub struct AgentRegistry {
    /// Map of agent ID to resolved agent config.
    pub(super) configs: DashMap<String, ResolvedAgentConfig>,
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
            configs: DashMap::new(),
        }
    }

    /// Populate the config store with the given resolved agent configs.
    /// Called once at startup by Daemon after ConfigManager loads agents.
    pub fn populate(&self, configs: Vec<ResolvedAgentConfig>) {
        for cfg in configs {
            self.configs.insert(cfg.id.clone(), cfg);
        }
    }

    /// Look up a resolved agent config by ID.
    /// Returns `None` if no config with the given ID exists.
    pub fn get(
        &self,
        id: &str,
    ) -> Option<dashmap::mapref::one::Ref<'_, String, ResolvedAgentConfig>> {
        self.configs.get(id)
    }

    /// Replace all stored configs with the given set (hot-reload).
    pub fn reload(&self, configs: Vec<ResolvedAgentConfig>) {
        self.configs.clear();
        for cfg in configs {
            self.configs.insert(cfg.id.clone(), cfg);
        }
    }

    /// Query the bootstrap mode for an agent by ID.
    ///
    /// Returns the agent's configured `BootstrapMode` (Full or Minimal).
    /// Returns `None` if the agent is not found in the registry.
    ///
    /// This is the System Prompt → AgentRegistry query path described in
    /// the design doc: System Prompt queries bootstrap mode configuration
    /// directly from AgentRegistry.
    pub fn query_bootstrap_mode(&self, agent_id: &str) -> Option<BootstrapMode> {
        self.configs.get(agent_id).map(|cfg| cfg.bootstrap_mode)
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
