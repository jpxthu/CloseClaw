//! System prompt builder trait and related types.
//!
//! Decouples the gateway from the concrete system prompt builder
//! implementation, allowing the builder to be swapped or mocked.

use std::path::Path;

use async_trait::async_trait;

use crate::bootstrap::BootstrapMode;
use crate::mode_transition::ModeTransition;
use crate::request_context::RequestContext;
use crate::session_mode::SessionMode;

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

// ── Dynamic prompt injection ───────────────────────────────────────────────

/// Bundles session state needed by [`DynamicPromptBuilder`].
///
/// Passed into [`DynamicPromptBuilder::build_prompt_parts`] so the
/// implementation can construct fresh dynamic sections without a
/// reverse dependency on the session crate.
pub struct DynamicPromptContext<'a> {
    /// The stored system prompt (may contain the boundary marker).
    pub system_prompt: Option<&'a str>,
    /// Current request metadata (sender, channel, timestamp).
    pub ctx: &'a RequestContext,
    /// Session working directory.
    pub workdir: &'a Path,
    /// Per-session append-section items.
    pub system_appends: &'a [String],
    /// Unix timestamp (seconds) when the session was created.
    pub session_created_at: i64,
    /// Current session mode (Normal / Plan / Auto).
    pub session_mode: SessionMode,
    /// Optional prompt overrides (agent / custom / override).
    pub overrides: Option<&'a PromptOverrides>,
    /// The user's original input text, used for plan-path analysis.
    pub user_input: Option<&'a str>,
    /// Pending mode transition to inject as a one-shot system prompt section.
    ///
    /// When `Some`, the dynamic builder pushes a `Section::ModeTransition`
    /// and clears the slot (one-shot injection).
    pub pending_mode_transition: Option<ModeTransition>,
}

/// Builder for the dynamic portion of the system prompt.
///
/// Called at LLM request time to produce fresh `system_static` and
/// `system_dynamic` values for [`InternalRequest`][crate::InternalRequest].
/// Implementations live in the `system_prompt` crate and are injected
/// into sessions by the gateway layer.
pub trait DynamicPromptBuilder: Send + Sync {
    /// Build `system_static` and `system_dynamic` for the current request.
    ///
    /// Returns `(system_static, system_dynamic)`. Either may be `None`.
    fn build_prompt_parts(
        &self,
        context: &DynamicPromptContext,
    ) -> (Option<String>, Option<String>);
}

/// Split a full system prompt into static and dynamic parts.
///
/// Uses the `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` marker as the split point:
///
/// - Content **before** the first marker → `Some(static)` (trailing whitespace trimmed)
/// - Content **after** the first marker → `Some(dynamic)` (leading whitespace trimmed)
/// - No marker → `(Some(full_prompt.to_owned()), None)`
/// - Empty string → `(None, None)`
pub fn split_static_dynamic(full_prompt: &str) -> (Option<String>, Option<String>) {
    if full_prompt.is_empty() {
        return (None, None);
    }
    let marker = "__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__";
    match full_prompt.find(marker) {
        Some(pos) => {
            let static_part = full_prompt[..pos].trim_end().to_owned();
            let dynamic_part = full_prompt[pos + marker.len()..].trim_start().to_owned();
            let s = if static_part.is_empty() {
                None
            } else {
                Some(static_part)
            };
            let d = if dynamic_part.is_empty() {
                None
            } else {
                Some(dynamic_part)
            };
            (s, d)
        }
        None => (Some(full_prompt.to_owned()), None),
    }
}
