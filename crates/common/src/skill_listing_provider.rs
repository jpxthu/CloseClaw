//! Skill listing provider trait for per-turn skill injection.
//!
//! Provides a focused interface for generating skill listing content
//! to be injected as a tool-role attachment each turn. Implemented by
//! a wrapper around `DiskSkillRegistry` in the daemon; consumed by
//! `ConversationSession` in the session crate.

/// Trait for generating formatted skill listings.
///
/// The session crate depends on this trait (defined in common) to
/// inject per-turn skill listings without requiring a direct
/// dependency on the skills crate.
pub trait SkillListingProvider: Send + Sync {
    /// Generate a formatted skill listing string for the given agent.
    ///
    /// When `agent_id` is provided, the listing is filtered to skills
    /// belonging to that agent. When `agent_skills` is provided, only
    /// skills whose names appear in the whitelist are included (unless
    /// the list is `["*"]`, which means no filtering).
    ///
    /// Returns an empty string if no skills match.
    fn generate_listing(&self, agent_id: Option<&str>, agent_skills: Option<&[String]>) -> String;
}
