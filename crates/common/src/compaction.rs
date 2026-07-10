//! Compaction configuration.

use serde::{Deserialize, Serialize};

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
