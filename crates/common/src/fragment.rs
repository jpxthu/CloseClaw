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
    /// Working directory of the agent — used by [`BootstrapFragmentProvider`]
    /// to locate bootstrap files.
    pub workdir: PathBuf,
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
            workdir: std::env::temp_dir(),
        }
    }
}

/// Trait for providers that contribute a section to the static layer of the
/// system prompt. The Builder collects registered providers, sorts them by
/// [`priority`](Self::priority), and concatenates their non-empty outputs.
///
/// The fragment type `F` is defined by the consumer crate (e.g.
/// `closeclaw-system-prompt`) and carries section classification and content.
#[async_trait]
pub trait PromptFragmentProvider<F: Send + Sync>: Send + Sync {
    /// Unique name of this provider, used for registration and logging.
    fn name(&self) -> &str;

    /// Lower values appear first in the assembled prompt.
    fn priority(&self) -> u32;

    /// Produce a prompt fragment for the given context.
    ///
    /// Returns `None` when there is nothing to contribute (e.g. no workspace
    /// directory, empty registry, missing file). The Builder silently skips
    /// `None` results.
    async fn generate(&self, ctx: &FragmentContext) -> Option<F>;

    /// Section-level cache key. Returns `None` when the section is not
    /// cacheable (e.g. registry-backed providers that manage their own
    /// invalidation).
    fn cache_key(&self, ctx: &FragmentContext) -> Option<String>;
}

#[cfg(test)]
#[path = "fragment_tests.rs"]
mod tests;
