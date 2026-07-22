//! Skill listing provider trait for per-turn skill injection.
//!
//! Provides a focused interface for generating skill listing content
//! to be injected as a tool-role attachment each turn. Implemented by
//! a wrapper around `DiskSkillRegistry` in the daemon; consumed by
//! `ConversationSession` in the session crate.

use std::path::PathBuf;

/// A matched conditional skill with its rendered listing line.
///
/// Returned by [`SkillListingProvider::find_conditional_matches`]
/// when a file path matches a conditional skill's glob patterns.
pub struct ConditionalSkillMatch {
    /// Skill name.
    pub name: String,
    /// Rendered listing line including the ⚡ auto-activates annotation,
    /// e.g. `- **foo**: desc — when ⚡ auto-activates on: *.rs`.
    pub listing_line: String,
}

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

    /// Generate a formatted skill listing **excluding** conditional skills
    /// (those with non-empty `paths`).
    ///
    /// Used for the initial turn and as the base for incremental diff
    /// computation. Conditional skills are injected separately via
    /// [`find_conditional_matches`].
    ///
    /// Returns an empty string if no non-conditional skills match.
    fn generate_listing_excluding_conditional(
        &self,
        agent_id: Option<&str>,
        agent_skills: Option<&[String]>,
    ) -> String;

    /// Find conditional skills whose glob patterns match the given file
    /// paths.
    ///
    /// Returns each matched skill as a [`ConditionalSkillMatch`] with a
    /// rendered listing line that includes the `⚡ auto-activates on:`
    /// annotation. Only skills with non-empty `paths` are considered.
    ///
    /// Returns an empty vec when `paths` is empty or no conditional
    /// skills match.
    fn find_conditional_matches(&self, paths: &[PathBuf]) -> Vec<ConditionalSkillMatch>;
}
