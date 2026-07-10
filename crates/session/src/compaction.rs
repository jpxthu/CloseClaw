//! Session Compaction Service
//!
//! Provides token estimation, auto-compaction threshold detection, and circuit breaker
//! for LLM context window management. This module contains only data types, pure
//! functions, and the `CompactionService` state machine. The actual LLM-calling
//! `execute_compact` function lives in the main crate's wrapper module.

pub use closeclaw_common::CompactConfig;
use closeclaw_common::RunningStats;

/// Simple message type for compaction operations.
///
/// This is a minimal representation of an LLM message, used by the session
/// crate to avoid depending on the full `llm::Message` type.
#[derive(Debug, Clone)]
pub struct CompactionMessage {
    /// Role of the message sender (e.g. "user", "assistant", "system").
    pub role: String,
    /// Text content of the message.
    pub content: String,
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

    /// Failed to parse summary from LLM response.
    #[error("Failed to parse summary from LLM response")]
    SummaryParseFailed,

    /// No messages provided for compaction.
    #[error("No messages provided for compaction")]
    EmptyMessages,
}

/// No-tools preamble constant.
pub const NO_TOOLS_PREAMBLE: &str = "You are a session summarizer. You must not call any tools or functions. You are analyzing a conversation session to create a summary. Output ONLY the <summary> tag with required content.";

/// Base compact prompt with 9-item summary structure.
pub const BASE_COMPACT_PROMPT: &str = "\n## Summary Structure\nYour summary must cover: 1) User Identity & Preferences, 2) Current Project & Context, 3) Key Decisions & Conclusions, 4) Open Questions & Unresolved Issues, 5) Technical State, 6) Conversation Flow, 7) Important Facts & References, 8) Agent Memory & Self-Knowledge, 9) Next Steps & Action Items.\n\n## Output Format\nWrite in English using bullet points. Be specific and concrete. Output ONLY: <summary>your summary here</summary>";

pub const NO_TOOLS_TRAILER: &str =
    "\n## Important\n- Do NOT call any tools. Output ONLY the <summary> tag.";

/// Model context window size table: (&str model_name, usize tokens).
/// Ordered by specificity: specific models first, then generic fallbacks.
pub const MODEL_CONTEXT_WINDOWS: &[(&str, usize)] = &[
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

/// Builds the compact prompt with optional custom instructions.
pub fn build_compact_prompt(custom_instructions: Option<&str>) -> String {
    let base = format!("{}\n{}", NO_TOOLS_PREAMBLE, BASE_COMPACT_PROMPT);
    match custom_instructions {
        Some(inst) if !inst.is_empty() => format!("{}\n\n保留 {}", base, inst),
        _ => format!("{}{}", base, NO_TOOLS_TRAILER),
    }
}

/// Extracts the `<summary>` content from an LLM response.
pub fn extract_summary(response: &str) -> Option<String> {
    let start_tag = "<summary>";
    let end_tag = "</summary>";
    let start = response.find(start_tag)?;
    let end = response.find(end_tag)?;
    if end <= start {
        return None;
    }
    Some(response[start + start_tag.len()..end].to_string())
}

/// Formats a boundary system message containing the summary.
///
/// Output format: `[Session Compaction | {trigger} | {timestamp}]\n\n{summary}`
/// where trigger is "手动压缩" or "自动压缩" and timestamp is UTC ISO 8601.
pub fn format_boundary_message(
    summary: &str,
    is_auto: bool,
    timestamp: chrono::DateTime<chrono::Utc>,
) -> String {
    let trigger = if is_auto {
        "自动压缩"
    } else {
        "手动压缩"
    };
    format!(
        "[Session Compaction | {} | {}]\n\n{}",
        trigger, timestamp, summary
    )
}

/// Estimate token count for a text string using character count coefficient.
pub fn estimate_tokens(text: &str, chars_per_token: f64) -> usize {
    if text.is_empty() {
        return 0;
    }
    let chars = text.chars().count();
    (chars as f64 * chars_per_token).ceil() as usize
}

/// Estimate total tokens for a slice of compaction messages.
pub fn estimate_messages_tokens(messages: &[CompactionMessage], chars_per_token: f64) -> usize {
    messages
        .iter()
        .map(|m| estimate_tokens(&m.content, chars_per_token))
        .sum()
}

/// Estimate total tokens combining precise RunningStats and character-based estimation.
///
/// When `stats.request_count > 0`, returns `stats.total_tokens` plus a character-based
/// estimate for the given messages. When `request_count == 0` (no LLM calls yet),
/// falls back to pure character-based estimation.
pub fn estimate_total_tokens(
    stats: &RunningStats,
    messages: &[CompactionMessage],
    chars_per_token: f64,
) -> usize {
    if stats.request_count > 0 {
        stats.total_tokens as usize + estimate_messages_tokens(messages, chars_per_token)
    } else {
        estimate_messages_tokens(messages, chars_per_token)
    }
}

/// Get the context window size for a model.
///
/// When `knowledge_context_window` is `Some(n)` and `n > 0`, returns `n`
/// (knowledge base value). Otherwise falls back to the hardcoded
/// [`MODEL_CONTEXT_WINDOWS`] table, defaulting to 128_000 for unknown models.
pub fn get_context_window(model: &str, knowledge_context_window: Option<u32>) -> usize {
    if let Some(kb_window) = knowledge_context_window {
        if kb_window > 0 {
            return kb_window as usize;
        }
    }
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
    pub fn token_warning_state(
        &self,
        used_tokens: usize,
        model: &str,
        knowledge_context_window: Option<u32>,
    ) -> TokenWarningState {
        let context_window = get_context_window(model, knowledge_context_window);
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
    pub fn percent_left(
        &self,
        used_tokens: usize,
        model: &str,
        knowledge_context_window: Option<u32>,
    ) -> usize {
        let context_window = get_context_window(model, knowledge_context_window);
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
    ///
    /// Delegates to [`token_warning_state`](Self::token_warning_state) and returns
    /// `true` only when the state is [`AutoCompactTriggered`](TokenWarningState::AutoCompactTriggered)
    /// and the circuit breaker has not tripped.
    pub fn should_auto_compact(
        &self,
        messages: &[CompactionMessage],
        model: &str,
        knowledge_context_window: Option<u32>,
        stats: &RunningStats,
    ) -> bool {
        if self.consecutive_failures >= self.config.max_consecutive_failures {
            return false;
        }
        let tokens = estimate_total_tokens(stats, messages, self.config.chars_per_token);
        matches!(
            self.token_warning_state(tokens, model, knowledge_context_window),
            TokenWarningState::AutoCompactTriggered
        )
    }

    /// Returns the number of consecutive compaction failures.
    pub fn consecutive_failures(&self) -> usize {
        self.consecutive_failures
    }

    /// Returns a reference to the compaction configuration.
    pub fn config(&self) -> &CompactConfig {
        &self.config
    }
}

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
