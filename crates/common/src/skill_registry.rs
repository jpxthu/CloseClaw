//! Skill registry trait for decoupling gateway from concrete skill implementation.
//!
//! Provides an interface for querying available skills without requiring
//! a direct dependency on the skills crate.

use async_trait::async_trait;

/// Trait for querying and managing skills.
///
/// Implemented by `DiskSkillRegistry` and `SkillRegistry` in the skills
/// crate; used by the gateway's session manager to list available skills
/// without a direct dependency on the skills module.
#[async_trait]
pub trait SkillRegistryQuery: Send + Sync {
    /// Check if a skill with the given name exists.
    async fn has_skill(&self, name: &str) -> bool;

    /// List all registered skill names.
    async fn list_skills(&self) -> Vec<String>;

    /// List skill names filtered by an optional agent-level whitelist.
    ///
    /// When `agent_skills` is `Some(["*"])` or `Some(empty)`, all skills
    /// are returned. When `Some(vec!["skill_a", "skill_b"])`, only matching
    /// skills are returned.
    async fn list_skills_for_agent(&self, agent_skills: Option<&[String]>) -> Vec<String>;

    /// Generate a formatted skill listing for system prompt injection.
    ///
    /// Returns an empty string when no skills match the filter.
    fn generate_listing(&self, agent_id: Option<&str>, agent_skills: Option<&[String]>) -> String;
}
