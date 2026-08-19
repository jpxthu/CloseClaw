//! Agent query traits for decoupling agent registry from skills/tools modules.
//!
//! Provides interfaces for querying agent skill and tool configurations
//! without requiring a direct dependency on the agent registry module.

use async_trait::async_trait;

/// Trait for querying agent skill configurations.
///
/// Implemented by `AgentRegistry` in the agent crate; used by the skills
/// crate to look up agent-level skill whitelists without depending on
/// the concrete registry type.
#[async_trait]
pub trait AgentSkillsQuery: Send + Sync {
    /// Get the effective skills list for an agent by ID.
    ///
    /// Returns `Some(skills)` if the agent exists and has a configured
    /// skills list, or `None` if not found. A `["*"]` or empty list
    /// means all skills are available.
    fn get_agent_skills(&self, agent_id: &str) -> Option<Vec<String>>;
}

/// The result of looking up an agent's tool configuration.
///
/// Returned by [`AgentToolsConfigQuery::get_agent_tools_config`].
/// Contains the effective tool whitelist and blacklist for an agent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentToolsConfig {
    /// Agent-level tool whitelist. `None` or `Some(["*"])` means all tools.
    pub tools: Option<Vec<String>>,
    /// Agent-level tool blacklist. `None` means no blacklist.
    pub disallowed_tools: Option<Vec<String>>,
}

/// Trait for querying agent-level tool filtering configuration.
///
/// Implemented by `AgentRegistry` in the agent crate; used by the tools
/// crate's `ToolRegistry` to query agent tool config without depending
/// on the concrete `AgentRegistry` type.
#[async_trait]
pub trait AgentToolsConfigQuery: Send + Sync {
    /// Get the effective tool whitelist and blacklist for an agent.
    ///
    /// Returns `None` if the agent is not found (no filtering — all tools allowed).
    async fn get_agent_tools_config(&self, agent_id: &str) -> Option<AgentToolsConfig>;
}
