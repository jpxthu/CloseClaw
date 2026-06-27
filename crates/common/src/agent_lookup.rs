//! Agent lookup trait for decoupling gateway from agent registry.
//!
//! Provides an interface for querying agent configuration without
//! requiring a direct dependency on the agent crate.

use async_trait::async_trait;

/// Trait for looking up agent configuration and registry data.
///
/// Implemented by `AgentRegistry` in the main crate; used by the gateway
/// crate to avoid a direct dependency on the agent module.
#[async_trait]
pub trait AgentLookup: Send + Sync {
    /// Look up an agent's model configuration by agent ID.
    ///
    /// Returns `Some(model_name)` if the agent has a configured model,
    /// or `None` if not found or no model configured.
    async fn get_agent_model(&self, agent_id: &str) -> Option<String>;

    /// Check if an agent ID is valid (exists in the registry).
    async fn agent_exists(&self, agent_id: &str) -> bool;

    /// Get the parent agent ID for a given agent, if any.
    async fn get_parent_id(&self, agent_id: &str) -> Option<String>;
}
