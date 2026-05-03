//! Session Compaction Service
//!
//! Provides token estimation, auto-compaction threshold detection, and circuit breaker
//! for LLM context window management.

use crate::llm::Message;

/// Configuration for compaction behavior.
#[derive(Debug, Clone)]
pub struct CompactConfig {
    /// Characters per token (linear estimation coefficient).
    pub chars_per_token: f64,
    /// Buffer tokens reserved below context window before triggering auto-compact.
    pub auto_compact_buffer_tokens: usize,
    /// Maximum consecutive compaction failures before circuit breaker trips.
    pub max_consecutive_failures: usize,
}

impl Default for CompactConfig {
    fn default() -> Self {
        Self {
            chars_per_token: 0.25,
            auto_compact_buffer_tokens: 13_000,
            max_consecutive_failures: 3,
        }
    }
}

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
}

/// Token warning state for monitoring context window pressure.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenWarningState {
    /// Normal state — plenty of context room.
    Normal,
    /// Warning state — approaching high usage.
    Warning,
    /// Auto-compact triggered — compaction should run.
    AutoCompactTriggered,
    /// Blocking state — context window nearly full, blocking new requests.
    Blocking,
}

/// Model context window size table: (&str model_name, usize tokens).
/// Ordered by specificity: specific models first, then generic fallbacks.
const MODEL_CONTEXT_WINDOWS: &[(&str, usize)] = &[
    // MiniMax models (1M context)
    ("mini-max", 1_000_000),
    ("mini-max-reasoning", 1_000_000),
    // GLM models (256K context)
    ("glm-5.1", 256_000),
    ("glm-5", 256_000),
    ("glm-4", 256_000),
    ("glm-3", 128_000),
    // Unknown / fallback
    ("unknown", 128_000),
];

/// Estimate token count for a text string using character count coefficient.
pub fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let chars = text.chars().count();
    (chars as f64 * 0.25).ceil() as usize
}

/// Estimate total tokens for a slice of messages.
pub fn estimate_messages_tokens(messages: &[Message]) -> usize {
    messages.iter().map(|m| estimate_tokens(&m.content)).sum()
}

/// Get the context window size for a model.
/// Returns 128_000 for unknown models.
pub fn get_context_window(model: &str) -> usize {
    MODEL_CONTEXT_WINDOWS
        .iter()
        .find(|(name, _)| model.starts_with(name))
        .map(|(_, tokens)| *tokens)
        .unwrap_or(128_000)
}

/// Session compaction service with auto-trigger and circuit breaker.
#[derive(Debug, Clone)]
pub struct CompactionService {
    config: CompactConfig,
    consecutive_failures: usize,
}

impl CompactionService {
    /// Create a new CompactionService with the given config.
    pub fn new(config: CompactConfig) -> Self {
        Self {
            config,
            consecutive_failures: 0,
        }
    }

    /// Returns the token warning state based on current usage and model context window.
    pub fn token_warning_state(&self, used_tokens: usize, model: &str) -> TokenWarningState {
        let context_window = get_context_window(model);
        let remaining = context_window.saturating_sub(used_tokens);

        // Blocking: ≤ 3,000 tokens left
        if remaining <= 3_000 {
            return TokenWarningState::Blocking;
        }
        // Auto-compact triggered: ≤ 13,000 tokens left
        if remaining <= 13_000 {
            return TokenWarningState::AutoCompactTriggered;
        }
        // Warning: ≤ 20,000 tokens left
        if remaining <= 20_000 {
            return TokenWarningState::Warning;
        }
        TokenWarningState::Normal
    }

    /// Returns the percentage of context window remaining (0-100).
    pub fn percent_left(&self, used_tokens: usize, model: &str) -> usize {
        let context_window = get_context_window(model);
        if context_window == 0 {
            return 0;
        }
        (context_window.saturating_sub(used_tokens) * 100 / context_window).min(100)
    }

    /// Records a compaction failure, incrementing the consecutive failure counter.
    pub fn record_failure(&mut self) {
        self.consecutive_failures += 1;
    }

    /// Records a compaction success, resetting the consecutive failure counter.
    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    /// Returns whether auto-compaction should run based on token usage and circuit breaker.
    pub fn should_auto_compact(&self, messages: &[Message], model: &str) -> bool {
        if self.consecutive_failures >= self.config.max_consecutive_failures {
            return false;
        }
        let tokens = estimate_messages_tokens(messages);
        let threshold =
            get_context_window(model).saturating_sub(self.config.auto_compact_buffer_tokens);
        tokens >= threshold
    }

    /// Returns the number of consecutive compaction failures.
    pub fn consecutive_failures(&self) -> usize {
        self.consecutive_failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens_english() {
        // "hello" = 5 chars * 0.25 = 1.25 -> ceil = 2
        let tokens = estimate_tokens("hello");
        assert!(tokens >= 2 && tokens <= 5, "expected 2-5, got {}", tokens);
    }

    #[test]
    fn test_estimate_tokens_chinese() {
        // "你好" = 2 chars * 0.25 = 0.5 -> ceil = 1
        let tokens = estimate_tokens("你好");
        assert!(tokens >= 1 && tokens <= 4, "expected 1-4, got {}", tokens);
    }

    #[test]
    fn test_estimate_tokens_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn test_estimate_tokens_emoji() {
        // "🎉🎊🔥" = 3 chars * 0.25 = 0.75 -> ceil = 1
        let tokens = estimate_tokens("🎉🎊🔥");
        assert!(tokens >= 1 && tokens <= 4, "expected 1-4, got {}", tokens);
    }

    #[test]
    fn test_estimate_tokens_long_string() {
        let s = "a".repeat(1000);
        assert_eq!(estimate_tokens(&s), 250);
    }

    #[test]
    fn test_get_context_window_minimax() {
        assert_eq!(get_context_window("mini-max"), 1_000_000);
    }

    #[test]
    fn test_get_context_window_glm() {
        assert_eq!(get_context_window("glm-5.1"), 256_000);
    }

    #[test]
    fn test_get_context_window_unknown() {
        assert_eq!(get_context_window("unknown-model-xyz"), 128_000);
    }

    #[test]
    fn test_should_auto_compact_below_threshold() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        let msgs = vec![Message {
            role: "user".to_string(),
            content: "short".to_string(),
        }];
        assert!(!service.should_auto_compact(&msgs, "mini-max"));
    }

    #[test]
    fn test_should_auto_compact_circuit_breaker() {
        let mut config = CompactConfig::default();
        config.max_consecutive_failures = 3;
        let mut service = CompactionService::new(config);
        // Record failures up to max
        service.record_failure();
        service.record_failure();
        service.record_failure();
        let msgs = vec![Message {
            role: "user".to_string(),
            content: "x".repeat(900_000),
        }];
        // Circuit breaker should trip
        assert!(!service.should_auto_compact(&msgs, "mini-max"));
    }

    #[test]
    fn test_token_warning_state_normal() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // 1,000,000 - 50,000 = 950,000 used -> 50,000 remaining > 20,000
        assert_eq!(
            service.token_warning_state(950_000, "mini-max"),
            TokenWarningState::Normal
        );
    }

    #[test]
    fn test_token_warning_state_warning() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // remaining = 20,000 -> Warning
        assert_eq!(
            service.token_warning_state(980_000, "mini-max"),
            TokenWarningState::Warning
        );
    }

    #[test]
    fn test_token_warning_state_auto_compact() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // remaining = 13,000 -> AutoCompactTriggered
        assert_eq!(
            service.token_warning_state(987_000, "mini-max"),
            TokenWarningState::AutoCompactTriggered
        );
    }

    #[test]
    fn test_token_warning_state_blocking() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // remaining = 3,000 -> Blocking
        assert_eq!(
            service.token_warning_state(997_000, "mini-max"),
            TokenWarningState::Blocking
        );
    }

    #[test]
    fn test_percent_left_normal() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        assert_eq!(service.percent_left(500_000, "mini-max"), 50);
    }

    #[test]
    fn test_percent_left_zero_used() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        assert_eq!(service.percent_left(0, "mini-max"), 100);
    }

    #[test]
    fn test_percent_left_near_full() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        assert_eq!(service.percent_left(999_000, "mini-max"), 0);
    }

    #[test]
    fn test_record_failure_increments() {
        let mut config = CompactConfig::default();
        config.max_consecutive_failures = 3;
        let mut service = CompactionService::new(config);
        assert_eq!(service.consecutive_failures(), 0);
        service.record_failure();
        assert_eq!(service.consecutive_failures(), 1);
        service.record_failure();
        assert_eq!(service.consecutive_failures(), 2);
    }

    #[test]
    fn test_record_success_resets() {
        let mut config = CompactConfig::default();
        config.max_consecutive_failures = 3;
        let mut service = CompactionService::new(config);
        service.record_failure();
        service.record_failure();
        assert_eq!(service.consecutive_failures(), 2);
        service.record_success();
        assert_eq!(service.consecutive_failures(), 0);
    }

    #[test]
    fn test_should_auto_compact_recovers_after_success() {
        let mut config = CompactConfig::default();
        config.max_consecutive_failures = 3;
        let mut service = CompactionService::new(config);
        // Push to edge of circuit breaker (3 failures = trip)
        service.record_failure();
        service.record_failure();
        service.record_failure(); // Now consecutive_failures=3 >= max=3, CB trips
                                  // 4M chars -> 1M tokens, exceeds 987K threshold
        let msgs = vec![Message {
            role: "user".to_string(),
            content: "x".repeat(4_000_000),
        }];
        assert!(!service.should_auto_compact(&msgs, "mini-max")); // CB trips
                                                                  // Success resets
        service.record_success();
        assert!(service.should_auto_compact(&msgs, "mini-max")); // CB recovers
    }
}
