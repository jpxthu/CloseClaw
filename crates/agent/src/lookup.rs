//! Agent lookup traits for decoupling gateway and tools from agent registry.
//!
//! Provides interfaces for querying agent configuration without
//! requiring a direct dependency on the concrete `AgentRegistry`.

use async_trait::async_trait;
use closeclaw_config::agents::ModelSpec;
use std::path::PathBuf;

/// Minimal agent config info needed by tools.
///
/// This is NOT the full `ResolvedAgentConfig` — it's a subset that
/// tools actually need, defined here to avoid a circular dependency
/// on `closeclaw-config`.
#[derive(Debug, Clone, Default)]
pub struct AgentConfigInfo {
    /// Agent's configured subagents model override, if any.
    pub subagents_model: Option<ModelSpec>,
}

/// Trait for looking up agent configuration.
///
/// Implemented by `AgentRegistry` in the main crate; used by the tools
/// crate's `SessionsSpawnTool` to look up parent agent config.
#[async_trait]
pub trait AgentConfigLookup: Send + Sync {
    /// Look up minimal agent config info by agent ID.
    ///
    /// Returns `Some(info)` if the agent exists, or `None` if not found.
    async fn lookup_agent_config(&self, agent_id: &str) -> Option<AgentConfigInfo>;
}

/// Trait for looking up agent configuration and registry data.
///
/// Implemented by `AgentRegistry` in the main crate; used by the gateway
/// crate to avoid a direct dependency on the agent module.
#[async_trait]
pub trait AgentLookup: Send + Sync {
    /// Look up an agent's model configuration by agent ID.
    ///
    /// Returns `Some(model_spec)` if the agent has a configured model,
    /// or `None` if not found or no model configured.
    async fn get_agent_model(&self, agent_id: &str) -> Option<ModelSpec>;

    /// Check if an agent ID is valid (exists in the registry).
    async fn agent_exists(&self, agent_id: &str) -> bool;

    /// Look up an agent's per-agent workspace path by agent ID.
    ///
    /// Returns `Some(path)` if the agent has a configured workspace,
    /// or `None` if not found or no workspace configured.
    async fn get_agent_workspace(&self, agent_id: &str) -> Option<PathBuf>;
}

/// Combined query trait for agent registry lookups.
///
/// Inherits [`AgentLookup`], [`AgentSkillsQuery`], and
/// [`AgentToolsConfigQuery`] so that a single `Arc<dyn AgentRegistryQuery>`
/// can satisfy all downstream query needs without multiple trait objects.
pub trait AgentRegistryQuery:
    AgentLookup
    + crate::skills_query::AgentSkillsQuery
    + crate::tools_config_query::AgentToolsConfigQuery
{
}
