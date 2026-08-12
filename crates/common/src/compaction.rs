//! Compaction configuration.

use serde::{Deserialize, Serialize};

// ── Compaction types ──────────────────────────────────────────────────

/// Result of a compaction operation.
#[derive(Debug, Clone)]
pub struct CompactionResult {
    /// Whether compaction was performed.
    pub performed: bool,
    /// Number of tokens in the original session.
    pub original_tokens: usize,
    /// Number of tokens after compaction (meaningful only if performed=true).
    pub compacted_tokens: usize,
    /// Human-readable message describing the outcome.
    pub message: String,
    /// Character count before compaction.
    pub before_char_count: usize,
    /// Character count after compaction.
    pub after_char_count: usize,
    /// Token count before compaction.
    pub before_token_count: usize,
    /// Token count after compaction.
    pub after_token_count: usize,
    /// Boundary system message containing the summary.
    pub boundary_message: String,
    /// Whether this compaction was triggered automatically.
    pub is_auto: bool,
}

/// Errors that can occur during compaction.
#[derive(Debug, thiserror::Error)]
pub enum CompactionError {
    /// LLM call failed.
    #[error("LLM call failed: {0}")]
    LLMCallFailed(String),

    /// Session not found.
    #[error("session not found: {0}")]
    SessionNotFound(String),

    /// Failed to parse summary from LLM response.
    #[error("Failed to parse summary from LLM response")]
    SummaryParseFailed,

    /// No messages provided for compaction.
    #[error("No messages provided for compaction")]
    EmptyMessages,

    /// Required handler not available.
    #[error("handler not available: {0}")]
    HandlerNotAvailable(String),
}

/// Configuration for compaction behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactConfig {
    /// Characters per token (linear estimation coefficient).
    pub chars_per_token: f64,
    /// Auto-compact trigger threshold as a fraction of context window (e.g. 0.05 = 5%).
    #[serde(default = "default_auto_compact_threshold_pct")]
    pub auto_compact_threshold_pct: f64,
    /// Warning threshold as a fraction of context window (e.g. 0.10 = 10%).
    #[serde(default = "default_warning_threshold_pct")]
    pub warning_threshold_pct: f64,
    /// Maximum consecutive compaction failures before circuit breaker trips.
    pub max_consecutive_failures: usize,
    /// Maximum number of history messages to keep before compaction.
    /// `None` means no truncation.
    #[serde(default)]
    pub max_history_messages: Option<usize>,
}

fn default_auto_compact_threshold_pct() -> f64 {
    0.05
}

fn default_warning_threshold_pct() -> f64 {
    0.10
}

impl CompactConfig {
    /// Validate configuration values.
    ///
    /// Checks:
    /// - Both percentage thresholds are in [0, 1].
    /// - `chars_per_token` is positive.
    /// - `auto_compact_threshold_pct` <= `warning_threshold_pct`.
    pub fn validate(&self) -> Result<(), String> {
        if self.chars_per_token <= 0.0 {
            return Err(format!(
                "chars_per_token ({}) must be positive",
                self.chars_per_token
            ));
        }
        if !(0.0..=1.0).contains(&self.auto_compact_threshold_pct) {
            return Err(format!(
                "auto_compact_threshold_pct ({}) must be in [0, 1]",
                self.auto_compact_threshold_pct
            ));
        }
        if !(0.0..=1.0).contains(&self.warning_threshold_pct) {
            return Err(format!(
                "warning_threshold_pct ({}) must be in [0, 1]",
                self.warning_threshold_pct
            ));
        }
        if self.auto_compact_threshold_pct > self.warning_threshold_pct {
            return Err(format!(
                "auto_compact_threshold_pct ({}) must be <= \
                 warning_threshold_pct ({})",
                self.auto_compact_threshold_pct,
                self.warning_threshold_pct
            ));
        }
        Ok(())
    }
}

impl Default for CompactConfig {
    fn default() -> Self {
        Self {
            chars_per_token: 0.25,
            auto_compact_threshold_pct: 0.05,
            warning_threshold_pct: 0.10,
            max_consecutive_failures: 3,
            max_history_messages: None,
        }
    }
}
