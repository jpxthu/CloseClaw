//! Shared hook configuration types for cross-module use.
//!
//! These skeleton types live in `closeclaw-common` (Layer 0) so both
//! `closeclaw-config` (Layer 1) and `closeclaw-session` (Layer 2) can
//! reference them without violating the dependency layering rules.
//!
//! Design reference: `docs/design/session/run-health.md`.

use serde::{Deserialize, Serialize};

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
}

fn default_true() -> bool {
    true
}

impl Default for HookConfig {
    fn default() -> Self {
        Self {
            hook_type: HookType::PlanCheck,
            enabled: true,
        }
    }
}
