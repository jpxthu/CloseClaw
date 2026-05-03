//! Session Compaction Service
//!
//! Provides token estimation, auto-compaction threshold detection, and circuit breaker
//! for LLM context window management.

use crate::llm::Message;
use crate::llm::{ChatRequest, LLMProvider};

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
    LLMCallFailed(#[from] crate::llm::LLMError),

    /// Failed to parse summary from LLM response.
    #[error("Failed to parse summary from LLM response")]
    SummaryParseFailed,

    /// No messages provided for compaction.
    #[error("No messages provided for compaction")]
    EmptyMessages,
}

/// No-tools preamble constant.
const NO_TOOLS_PREAMBLE: &str = "You are a session summarizer. You must not call any tools or functions. You are analyzing a conversation session to create a summary. Output ONLY the <summary> tag with required content.";

/// Base compact prompt with 9-item summary structure.
const BASE_COMPACT_PROMPT: &str = "\n## Summary Structure\nYour summary must cover: 1) User Identity & Preferences, 2) Current Project & Context, 3) Key Decisions & Conclusions, 4) Open Questions & Unresolved Issues, 5) Technical State, 6) Conversation Flow, 7) Important Facts & References, 8) Agent Memory & Self-Knowledge, 9) Next Steps & Action Items.\n\n## Output Format\nWrite in English using bullet points. Be specific and concrete. Output ONLY: <summary>your summary here</summary>";

const NO_TOOLS_TRAILER: &str =
    "\n## Important\n- Do NOT call any tools. Output ONLY the <summary> tag.";

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
pub fn format_boundary_message(summary: &str, is_auto: bool) -> String {
    let trigger = if is_auto {
        "自动压缩"
    } else {
        "手动压缩"
    };
    format!("[Session Compaction | {}]\n\n{}", trigger, summary)
}

/// Executes session compaction: builds prompt, calls LLM, parses summary, formats result.
///
/// # Arguments
/// * `messages` - Session messages to compact
/// * `llm` - LLM provider
/// * `model_name` - Model name for the request
/// * `custom_instructions` - Optional custom instructions appended to the prompt
/// * `is_auto` - Whether this is an automatic compaction
///
/// # Errors
/// * `EmptyMessages` - No messages provided
/// * `LLMCallFailed` - LLM call returned an error
/// * `SummaryParseFailed` - LLM response contains no `<summary>` tag
pub async fn execute_compact(
    messages: &[Message],
    llm: &dyn LLMProvider,
    model_name: &str,
    custom_instructions: Option<&str>,
    is_auto: bool,
) -> Result<CompactionResult, CompactionError> {
    if messages.is_empty() {
        return Err(CompactionError::EmptyMessages);
    }

    let before_char_count: usize = messages.iter().map(|m| m.content.chars().count()).sum();
    let before_token_count = estimate_messages_tokens(messages);

    // Build system prompt with compaction instructions
    let compact_prompt = build_compact_prompt(custom_instructions);
    let system_message = Message {
        role: "system".to_string(),
        content: compact_prompt,
    };

    let request = ChatRequest {
        model: model_name.to_string(),
        messages: vec![system_message],
        temperature: 0.3,
        max_tokens: Some(8192),
    };

    let response = llm
        .chat(request)
        .await
        .map_err(CompactionError::LLMCallFailed)?;

    let summary = match extract_summary(&response.content) {
        Some(s) => s,
        None => return Err(CompactionError::SummaryParseFailed),
    };

    let boundary_message = format_boundary_message(&summary, is_auto);
    let after_char_count = boundary_message.chars().count();
    let after_token_count = estimate_tokens(&boundary_message);

    Ok(CompactionResult {
        performed: true,
        original_tokens: before_token_count,
        compacted_tokens: after_token_count,
        message: format!(
            "Compacted {} messages ({} chars, {} tokens) → {} chars, {} tokens",
            messages.len(),
            before_char_count,
            before_token_count,
            after_char_count,
            after_token_count
        ),
        before_char_count,
        after_char_count,
        before_token_count,
        after_token_count,
        boundary_message,
        is_auto,
    })
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
