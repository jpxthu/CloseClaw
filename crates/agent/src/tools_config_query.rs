//! Agent tools config query trait for decoupling tools from agent registry.
//!
//! Provides an interface for querying agent-level tool filtering
//! configuration without requiring a direct dependency on the agent module.

use async_trait::async_trait;

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
/// Implemented by `AgentRegistry` in the main crate; used by the tools
/// crate's `ToolRegistry` to query agent tool config without depending
/// on the concrete `AgentRegistry` type.
#[async_trait]
pub trait AgentToolsConfigQuery: Send + Sync {
    /// Get the effective tool whitelist and blacklist for an agent.
    ///
    /// Returns `None` if the agent is not found (no filtering — all tools allowed).
    async fn get_agent_tools_config(&self, agent_id: &str) -> Option<AgentToolsConfig>;
}
