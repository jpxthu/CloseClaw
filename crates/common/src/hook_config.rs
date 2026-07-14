//! Shared hook configuration types for cross-module use.
//!
//! These skeleton types live in `closeclaw-common` (Layer 0) so both
//! `closeclaw-config` (Layer 1) and `closeclaw-session` (Layer 2) can
//! reference them without violating the dependency layering rules.
//!
//! Design reference: `docs/design/session/run-health.md`.

use serde::{Deserialize, Serialize};

/// Tunable parameters for hook behaviour.
///
/// Each field corresponds to a threshold or minimum that was
/// previously hard-coded inside prompt templates. Making them
/// configurable lets agent profiles adjust sensitivity without
/// code changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookParams {
    /// How many consecutive similar tool calls constitute a loop.
    /// Used by `LoopCheck`.
    #[serde(default = "default_loop_check_repetition_threshold")]
    pub loop_check_repetition_threshold: usize,
    /// Minimum number of tool calls in a turn before
    /// `ProgressCheck` considers the turn eligible for review.
    #[serde(default = "default_progress_check_min_tool_calls")]
    pub progress_check_min_tool_calls: usize,
}

fn default_loop_check_repetition_threshold() -> usize {
    3
}

fn default_progress_check_min_tool_calls() -> usize {
    1
}

impl Default for HookParams {
    fn default() -> Self {
        Self {
            loop_check_repetition_threshold: default_loop_check_repetition_threshold(),
            progress_check_min_tool_calls: default_progress_check_min_tool_calls(),
        }
    }
}

/// Types of quality gate hooks.
///
/// Each variant identifies a specific review mechanism that can be
/// configured per-agent. Prompt templates and detection logic live
/// in the session crate's `hook_reviewer` module.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HookType {
    /// Detects turns where the LLM only plans/promises without executing.
    PlanCheck,
    /// Detects repeated tool calls with similar parameters (loops).
    LoopCheck,
    /// Detects turns with no verifiable progress.
    ProgressCheck,
}

/// Configuration for a single hook.
///
/// Used in agent config files (`config.json`) and consumed by the
/// session crate's `HookReviewer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookConfig {
    /// Which type of hook to run.
    pub hook_type: HookType,
    /// Whether this hook is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Tunable parameters that control hook behaviour.
    /// Defaults are chosen so that omitting `params` from JSON
    /// preserves the previous hard-coded behaviour.
    #[serde(default)]
    pub params: HookParams,
}

fn default_true() -> bool {
    true
}

impl Default for HookConfig {
    fn default() -> Self {
        Self {
            hook_type: HookType::PlanCheck,
            enabled: true,
            params: HookParams::default(),
        }
    }
}
