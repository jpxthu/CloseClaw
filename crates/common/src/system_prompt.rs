//! System prompt builder trait and related types.
//!
//! Decouples the gateway from the concrete system prompt builder
//! implementation, allowing the builder to be swapped or mocked.

use async_trait::async_trait;

use crate::bootstrap::BootstrapMode;

/// Overrides for the three-tier priority prompt system.
///
/// When resolving the final system prompt, the caller checks these in order:
///   1. `override_prompt` — highest priority, replaces the entire static layer
///   2. `agent_prompt`    — agent-level prompt
///   3. `custom_prompt`   — user-defined custom prompt
///
/// If none is set, the normal section-based rendering is used.
#[derive(Debug, Clone, Default)]
pub struct PromptOverrides {
    pub override_prompt: Option<String>,
    pub agent_prompt: Option<String>,
    pub custom_prompt: Option<String>,
}

/// Trait for building system prompts.
///
/// Implemented by the concrete builder in the main crate; used by
/// session handlers to generate system prompts without a direct
/// dependency on the system_prompt module.
#[async_trait]
pub trait SystemPromptBuilder: Send + Sync {
    /// Build a complete system prompt for the given session.
    ///
    /// # Arguments
    /// * `session_id` — the session requesting the prompt
    /// * `agent_id` — the agent whose prompt to build
    /// * `overrides` — optional priority prompt overrides
    ///
    /// Returns the rendered system prompt string.
    async fn build_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        overrides: Option<&PromptOverrides>,
        bootstrap_mode_override: Option<BootstrapMode>,
    ) -> String;

    /// Invalidate cached prompt sections.
    ///
    /// Called when workspace files, tools, or skills change.
    async fn invalidate_cache(&self);
}
