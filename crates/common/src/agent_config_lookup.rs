//! Agent config lookup trait for decoupling tools from agent registry.
//!
//! Provides an interface for looking up agent configurations without
//! requiring a direct dependency on the concrete `AgentRegistry`.

use async_trait::async_trait;

/// Minimal agent config info needed by tools.
///
/// This is NOT the full `ResolvedAgentConfig` — it's a subset that
/// tools actually need, defined here to avoid a circular dependency
/// on `closeclaw-config`.
#[derive(Debug, Clone, Default)]
pub struct AgentConfigInfo {
    /// Agent's configured subagents model override, if any.
    pub subagents_model: Option<String>,
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
