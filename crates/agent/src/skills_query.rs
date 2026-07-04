//! Agent skills query trait for decoupling skills from agent registry.
//!
//! Provides an interface for querying agent skill configurations without
//! requiring a direct dependency on the agent registry module.

use async_trait::async_trait;

/// Trait for querying agent skill configurations.
///
/// Implemented by `AgentRegistry` in the main crate; used by the skills
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
