//! Agent Registry - stores resolved agent configurations.
//!
//! This module is a pure configuration layer: it holds the resolved configs
//! populated once at startup (or reloaded at runtime) and exposes read-only
//! queries. All runtime state (processes, lifecycle) lives elsewhere.

use closeclaw_common::BootstrapMode;
use closeclaw_config::agents::ResolvedAgentConfig;
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
        Self::new()
    }
}

impl AgentRegistry {
    /// Create a new registry.
    pub fn new() -> Self {
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

    /// Iterate all registered agent configs.
    pub fn iter(&self) -> dashmap::iter::Iter<'_, String, ResolvedAgentConfig> {
        self.configs.iter()
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
pub fn create_registry() -> SharedAgentRegistry {
    Arc::new(AgentRegistry::new())
}

// ═══════════════════════════════════════════════════════════════════════════
// AgentLookup — bridge to closeclaw_common trait
// ═══════════════════════════════════════════════════════════════════════════

#[async_trait::async_trait]
impl closeclaw_common::agent_lookup::AgentLookup for AgentRegistry {
    async fn get_agent_model(&self, agent_id: &str) -> Option<String> {
        self.get(agent_id).and_then(|cfg| cfg.model.clone())
    }

    async fn agent_exists(&self, agent_id: &str) -> bool {
        self.get(agent_id).is_some()
    }

    async fn get_parent_id(&self, agent_id: &str) -> Option<String> {
        self.get(agent_id).and_then(|cfg| cfg.parent_id.clone())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// AgentSkillsQuery — bridge to closeclaw_common trait
// ═══════════════════════════════════════════════════════════════════════════

impl closeclaw_common::AgentSkillsQuery for AgentRegistry {
    fn get_agent_skills(&self, agent_id: &str) -> Option<Vec<String>> {
        self.get(agent_id).and_then(|cfg| cfg.effective_skills())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// AgentToolsConfigQuery — bridge to closeclaw_common trait
// ═══════════════════════════════════════════════════════════════════════════

#[async_trait::async_trait]
impl closeclaw_common::AgentToolsConfigQuery for AgentRegistry {
    async fn get_agent_tools_config(
        &self,
        agent_id: &str,
    ) -> Option<closeclaw_common::AgentToolsConfig> {
        self.get(agent_id).map(|cfg| {
            let tools = if cfg.tools.is_empty() || cfg.tools == ["*"] {
                None
            } else {
                Some(cfg.tools.clone())
            };
            let disallowed_tools = if cfg.disallowed_tools.is_empty() {
                None
            } else {
                Some(cfg.disallowed_tools.clone())
            };
            closeclaw_common::AgentToolsConfig {
                tools,
                disallowed_tools,
            }
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// AgentConfigLookup — bridge to closeclaw_common trait
// ═══════════════════════════════════════════════════════════════════════════

#[async_trait::async_trait]
impl closeclaw_common::AgentConfigLookup for AgentRegistry {
    async fn lookup_agent_config(
        &self,
        agent_id: &str,
    ) -> Option<closeclaw_common::AgentConfigInfo> {
        self.get(agent_id)
            .map(|cfg| closeclaw_common::AgentConfigInfo {
                subagents_model: cfg.subagents.model.clone(),
            })
    }
}

#[cfg(test)]
mod config_tests;
