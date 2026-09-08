//! System prompt builder trait and related types.
//!
//! Decouples the gateway from the concrete system prompt builder
//! implementation, allowing the builder to be swapped or mocked.

use std::path::Path;

use async_trait::async_trait;

use crate::bootstrap::BootstrapMode;
use crate::fragment::SessionRole;
use crate::request_context::RequestContext;
use crate::session_mode::SessionMode;

/// Mode transition type — signals which transition triggered prompt injection.
///
/// Used by [`DynamicPromptContext`] to communicate mode changes to the
/// system prompt builder, which injects the corresponding prompt from
/// design doc §6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeTransition {
    /// Re-entering Plan Mode after having previously exited it.
    PlanModeReentry,
    /// Exiting Plan Mode (user triggered execution).
    PlanModeExit,
    /// Exiting Auto Mode (auto execution completed).
    AutoModeExit,
}

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
        session_role: SessionRole,
    ) -> String;

    /// Build a system prompt including activated conditional skills.
    ///
    /// Same as [`build_prompt`](Self::build_prompt) but passes the
    /// activated skill set through to the provider pipeline via
    /// [`FragmentContext::activated_skills`]. This is the SP rebuild
    /// path: [`SkillsFragmentProvider`] reads the activation set to
    /// include activated conditional skills in the listing.
    ///
    /// The default implementation ignores the activated set and
    /// delegates to [`build_prompt`], matching the pre-activation
    /// behavior.
    async fn build_prompt_with_activated(
        &self,
        session_id: &str,
        agent_id: &str,
        overrides: Option<&PromptOverrides>,
        bootstrap_mode_override: Option<BootstrapMode>,
        activated_skills: Vec<String>,
        session_role: SessionRole,
    ) -> String {
        let _ = activated_skills;
        self.build_prompt(
            session_id,
            agent_id,
            overrides,
            bootstrap_mode_override,
            session_role,
        )
        .await
    }

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
    /// Whether the session context has been compacted (for sparse prompt injection).
    pub is_compacted: bool,
    /// Whether this prompt is for a sub-agent (for sub-agent sparse injection).
    pub is_sub_agent: bool,
    /// Whether the session has the git_status config switch enabled.
    ///
    /// When `true`, the dynamic builder may inject a GitStatus section
    /// if the working directory is a git repository.
    pub is_git_status_enabled: bool,
    /// Mode transition that triggered this prompt build.
    ///
    /// When `Some`, a mode transition prompt from design doc §6 is
    /// injected. `None` means no transition occurred on this request.
    pub mode_transition: Option<ModeTransition>,
    /// Plan file path for Auto Mode prompt injection.
    ///
    /// When `Some`, the dynamic builder reads the plan file content
    /// and injects it as a `## Plan File` section after the Mode
    /// Instruction in Auto Mode. `None` means no plan file context.
    pub plan_file_path: Option<&'a str>,
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
    ///
    /// Appends are merged into the dynamic field by the implementation
    /// before returning (two-field contract, see
    /// `docs/design/system_prompt/kv-cache.md`). The cache adapter
    /// receives only two fields: static and dynamic.
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
