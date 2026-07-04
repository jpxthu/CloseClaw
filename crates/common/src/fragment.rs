use std::path::PathBuf;

use async_trait::async_trait;

use crate::bootstrap::BootstrapMode;

/// Context passed to each [`PromptFragmentProvider`] during system prompt construction.
#[derive(Debug, Clone, Default)]
pub struct FragmentContext {
    /// Agent identifier — used by [`SkillsFragmentProvider`] to filter visible skills.
    pub agent_id: Option<String>,
    /// Bootstrap mode (Minimal / Full) — used by [`BootstrapFragmentProvider`]
    /// to select the file set. When `None`, falls back to [`AgentRegistry`] lookup.
    pub bootstrap_mode: Option<BootstrapMode>,
    /// Working directory of the agent — used by [`BootstrapFragmentProvider`]
    /// to locate bootstrap files. When `None`, bootstrap section is skipped.
    pub workdir: Option<PathBuf>,
    /// Agent directory — used by [`BootstrapFragmentProvider`] to locate
    /// bootstrap files. When `Some`, takes priority over [`workdir`](Self::workdir).
    /// When `None`, falls back to `workdir`.
    pub agent_dir: Option<PathBuf>,
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
