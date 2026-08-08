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
    /// Buffer tokens reserved below context window before triggering auto-compact.
    pub auto_compact_buffer_tokens: usize,
    /// Maximum consecutive compaction failures before circuit breaker trips.
    pub max_consecutive_failures: usize,
    /// Maximum number of history messages to keep before compaction.
    /// `None` means no truncation.
    #[serde(default)]
    pub max_history_messages: Option<usize>,
}

impl Default for CompactConfig {
    fn default() -> Self {
        Self {
            chars_per_token: 0.25,
            auto_compact_buffer_tokens: 13_000,
            max_consecutive_failures: 3,
            max_history_messages: None,
        }
    }
}
