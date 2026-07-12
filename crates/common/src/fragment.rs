use std::path::PathBuf;

use async_trait::async_trait;

use crate::bootstrap::BootstrapMode;

/// Context passed to each [`PromptFragmentProvider`] during system prompt construction.
#[derive(Debug, Clone)]
pub struct FragmentContext {
    /// Agent identifier — used by [`SkillsFragmentProvider`] to filter visible skills.
    pub agent_id: String,
    /// Bootstrap mode (Minimal / Full) — used by [`BootstrapFragmentProvider`]
    /// to select the file set.
    pub bootstrap_mode: BootstrapMode,
    /// Directory containing bootstrap files, used by [`BootstrapFragmentProvider`]
    /// to locate bootstrap files.
    pub bootstrap_dir: PathBuf,
    /// Effective spawn depth budget for the current session.
    ///
    /// When `Some(budget)` where `budget ≤ 0`, the `sessions_spawn`
    /// tool is filtered out of the visible tool list — the session
    /// cannot spawn further children (design doc §Depth 追踪).
    /// `None` means the budget is unknown (no filtering applied).
    pub effective_spawn_budget: Option<u32>,
}

impl FragmentContext {
    /// Returns a [`FragmentContext`] with reasonable defaults for unit tests.
    ///
    /// `#[cfg(test)]` is intentionally omitted because it does not propagate
    /// across crate boundaries — downstream crates (e.g. system_prompt) also
    /// need this constructor in their own test suites. The `#[doc(hidden)]`
    /// attribute keeps it out of generated docs to signal that this is a
    /// test-only helper, not a public API.
    #[doc(hidden)]
    pub fn test_default() -> Self {
        Self {
            agent_id: String::new(),
            bootstrap_mode: BootstrapMode::Full,
            bootstrap_dir: std::env::temp_dir(),
            effective_spawn_budget: None,
        }
    }
}

/// Section type classification for a prompt fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionType {
    /// Bootstrap files (agent profile, workspace rules, etc.)
    Bootstrap,
    /// Tool registry / tool listings
    Tools,
    /// Skill listings
    Skills,
    /// Long-term memory (MEMORY.md)
    Memory,
}

/// A single prompt fragment produced by a [`PromptFragmentProvider`].
#[derive(Debug, Clone)]
pub struct PromptFragment {
    /// Section title, e.g. `"## AGENTS.md"` or `"## Available Skills"`.
    pub section_title: String,
    /// Classification of the section content.
    pub section_type: SectionType,
    /// Rendered text content of the section.
    pub content: String,
}

/// Trait for providers that contribute a section to the static layer of the
/// system prompt. The Builder collects registered providers, sorts them by
/// [`priority`](Self::priority), and concatenates their non-empty outputs.
#[async_trait]
pub trait PromptFragmentProvider: Send + Sync {
    /// Unique name of this provider, used for registration and logging.
    fn name(&self) -> &str;

    /// Lower values appear first in the assembled prompt.
    fn priority(&self) -> u32;

    /// Produce a prompt fragment for the given context.
    ///
    /// Returns `None` when there is nothing to contribute (e.g. no workspace
    /// directory, empty registry, missing file). The Builder silently skips
    /// `None` results.
    async fn generate(&self, ctx: &FragmentContext) -> Option<PromptFragment>;

    /// Section-level cache key. Returns `None` when the section is not
    /// cacheable (e.g. registry-backed providers that manage their own
    /// invalidation).
    fn cache_key(&self, ctx: &FragmentContext) -> Option<String>;
}

#[cfg(test)]
#[path = "fragment_tests.rs"]
mod tests;
