//! Session Compaction Service
//!
//! Provides token estimation, auto-compaction threshold detection, and circuit breaker
//! for LLM context window management. This module contains data types, pure
//! functions, and the `CompactionService` state machine with its `compact` method
//! that executes compaction via an injected chat function.

pub use closeclaw_common::{CompactConfig, CompactionError, CompactionResult};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use closeclaw_common::RunningStats;
use closeclaw_debug_log::DebugLog;

use crate::debug_log::{self, SessionDebugLogContext};

/// Boxed async chat function for LLM injection.
///
/// Takes (model_name, messages) and returns (response_content, retries)
/// on success, or an error message on failure.
pub type ChatFn = Arc<
    dyn Fn(
            String,
            Vec<CompactionMessage>,
        ) -> Pin<Box<dyn Future<Output = Result<(String, u32), String>> + Send>>
        + Send
        + Sync,
>;

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

/// No-tools preamble constant.
pub const NO_TOOLS_PREAMBLE: &str = "You are a session summarizer. You must not call any tools or functions. You are analyzing a conversation session to create a summary. Output ONLY the <summary> tag with required content.";

/// Base compact prompt with 6-dimension summary structure.
pub const BASE_COMPACT_PROMPT: &str = "\n## Summary Structure\nYour summary must cover these 6 dimensions:\n1. Goal — the user's objective and what they are trying to achieve\n2. Constraints & Preferences — any constraints, preferences, or requirements the user has mentioned\n3. Progress — what has been done, what is in progress, and what is blocked\n4. Key Decisions — important decisions and their rationale\n5. Next Steps — planned follow-up actions\n6. Critical Context — any other context critical to continuing the conversation\n\n## Output Format\nWrite in English using bullet points. Be specific and concrete.\nFor Progress, label items as Done / In Progress / Blocked.\nOutput ONLY: <summary>your summary here</summary>";

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
/// Output format: `[Session Compaction | {trigger}] {summary}`
/// where trigger is "手动压缩" or "自动压缩".
pub fn format_boundary_message(summary: &str, is_auto: bool) -> String {
    let trigger = if is_auto {
        "自动压缩"
    } else {
        "手动压缩"
    };
    format!("[Session Compaction | {}] {}", trigger, summary)
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

/// Combine precise token count with character-based estimation for remaining messages.
///
/// When `precise_tokens` is `Some(count)` with `request_count > 0`, returns
/// `Some(count + estimated_for_remaining)` where remaining messages beyond
/// the counted set are estimated by character count. Returns `None` in all
/// other cases, letting the caller fall back to pure character-based
/// estimation.
fn combine_precise_and_estimated(
    precise_tokens: Option<usize>,
    request_count: u64,
    messages: &[CompactionMessage],
    chars_per_token: f64,
) -> Option<usize> {
    let precise = precise_tokens?;
    if request_count == 0 {
        return None;
    }
    let start = (request_count as usize).min(messages.len());
    let remaining_tokens: usize = messages[start..]
        .iter()
        .map(|m| estimate_tokens(&m.content, chars_per_token))
        .sum();
    Some(precise + remaining_tokens)
}

/// Estimate total tokens combining precise RunningStats and character-based estimation.
///
/// When `stats.request_count > 0`, returns `stats.total_tokens` plus a character-based
/// estimate for messages beyond the counted set (skipping the first `request_count`
/// messages whose tokens are already accounted for in `stats.total_tokens`).
/// When `request_count == 0` (no LLM calls yet), falls back to pure
/// character-based estimation.
pub fn estimate_total_tokens(
    stats: &RunningStats,
    messages: &[CompactionMessage],
    chars_per_token: f64,
) -> usize {
    combine_precise_and_estimated(
        Some(stats.total_tokens as usize),
        stats.request_count,
        messages,
        chars_per_token,
    )
    .unwrap_or_else(|| estimate_messages_tokens(messages, chars_per_token))
}

/// Compute the token count before compaction using precise stats when available.
///
/// When `stats` is provided with `request_count > 0`, returns
/// `stats.total_tokens` (precise usage from completed API calls) plus
/// a character-based estimate for messages beyond the counted set.
/// When `stats` is `None` or `request_count == 0`, falls back to
/// pure character-based estimation for all messages.
///
/// This is the single source of truth for before-compaction token
/// counting, shared by [`CompactionService::compact`].
pub fn compute_before_tokens(
    messages: &[CompactionMessage],
    stats: Option<&RunningStats>,
    chars_per_token: f64,
) -> usize {
    stats
        .and_then(|s| {
            combine_precise_and_estimated(
                Some(s.total_tokens as usize),
                s.request_count,
                messages,
                chars_per_token,
            )
        })
        .unwrap_or_else(|| estimate_messages_tokens(messages, chars_per_token))
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
    ///
    /// # Panics
    ///
    /// Panics if `config.auto_compact_threshold_pct > config.warning_threshold_pct`.
    pub fn new(config: CompactConfig) -> Self {
        config.validate().expect("CompactConfig validation failed");
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

        let compact_threshold =
            (context_window as f64 * self.config.auto_compact_threshold_pct).ceil() as usize;
        let warning_threshold =
            (context_window as f64 * self.config.warning_threshold_pct).ceil() as usize;

        // Auto-compact triggered: ≤ auto_compact_threshold_pct of context window left
        if remaining <= compact_threshold {
            return TokenWarningState::AutoCompactTriggered;
        }
        // Warning: ≤ warning_threshold_pct of context window left
        if remaining <= warning_threshold {
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

    /// Executes a compaction: calls the LLM to summarize the conversation,
    /// returns the compaction result with the boundary message.
    ///
    /// The LLM is injected via `chat_fn` to avoid depending on the `llm` crate.
    /// When `stats` is provided with `request_count > 0`, uses
    /// `stats.total_tokens` for precise token counting. Falls back to
    /// pure character estimation when `None`.
    ///
    /// On success, resets `consecutive_failures` to 0.
    /// The reply message format is: `"压缩完成：{before_token_count} → {after_token_count} tokens"`
    /// to align with the design document's requirement of showing token counts.
    pub async fn compact(
        &mut self,
        messages: &[CompactionMessage],
        model: &str,
        instruction: Option<&str>,
        is_auto: bool,
        stats: Option<&RunningStats>,
        chat_fn: &ChatFn,
    ) -> Result<CompactionResult, CompactionError> {
        self.compact_with_debug_log(
            messages,
            model,
            instruction,
            is_auto,
            stats,
            chat_fn,
            None,
            "",
            None,
        )
        .await
    }

    /// Executes a compaction with optional debug-log emission.
    ///
    /// Same as [`compact`](Self::compact) but emits structured debug-log events
    /// when `debug_log` is `Some` in the params.
    #[allow(clippy::too_many_arguments)]
    pub async fn compact_with_debug_log(
        &mut self,
        messages: &[CompactionMessage],
        model: &str,
        instruction: Option<&str>,
        is_auto: bool,
        stats: Option<&RunningStats>,
        chat_fn: &ChatFn,
        debug_log: Option<&DebugLog>,
        trace_id: &str,
        session_key: Option<&str>,
    ) -> Result<CompactionResult, CompactionError> {
        if messages.is_empty() {
            return Err(CompactionError::EmptyMessages);
        }

        // Emit debug-log: compaction started.
        debug_log::emit_session_event(debug_log::SessionEmitEventParams {
            ctx: SessionDebugLogContext::new(debug_log, trace_id, session_key),
            level: closeclaw_debug_log::LogLevel::Info,
            source_module: "session",
            event_type: "session.compaction.started",
            payload: serde_json::json!({
                "model": model,
                "is_auto": is_auto,
                "message_count": messages.len(),
            }),
            parent: None,
        });

        let prompt = build_compact_prompt(instruction);
        let mut llm_messages = vec![CompactionMessage {
            role: "system".to_string(),
            content: prompt,
        }];
        for m in messages {
            llm_messages.push(CompactionMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            });
        }

        let (response_content, _retries) =
            chat_fn(model.to_string(), llm_messages)
                .await
                .map_err(|e| {
                    // Emit debug-log: compaction failed.
                    debug_log::emit_session_event(debug_log::SessionEmitEventParams {
                        ctx: SessionDebugLogContext::new(debug_log, trace_id, session_key),
                        level: closeclaw_debug_log::LogLevel::Error,
                        source_module: "session",
                        event_type: "session.compaction.failed",
                        payload: serde_json::json!({
                            "model": model,
                            "is_auto": is_auto,
                            "error": e,
                        }),
                        parent: None,
                    });
                    CompactionError::LLMCallFailed(e)
                })?;

        let summary =
            extract_summary(&response_content).ok_or(CompactionError::SummaryParseFailed)?;

        let boundary = format_boundary_message(&summary, is_auto);
        let before_chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();
        let before_tokens = compute_before_tokens(messages, stats, self.config.chars_per_token);
        let after_tokens = estimate_tokens(&boundary, self.config.chars_per_token);
        let after_chars = boundary.chars().count();

        self.record_success();

        // Emit debug-log: compaction completed.
        debug_log::emit_session_event(debug_log::SessionEmitEventParams {
            ctx: SessionDebugLogContext::new(debug_log, trace_id, session_key),
            level: closeclaw_debug_log::LogLevel::Info,
            source_module: "session",
            event_type: "session.compaction.completed",
            payload: serde_json::json!({
                "model": model,
                "is_auto": is_auto,
                "before_tokens": before_tokens,
                "after_tokens": after_tokens,
                "before_chars": before_chars,
                "after_chars": after_chars,
            }),
            parent: None,
        });

        Ok(CompactionResult {
            performed: true,
            original_tokens: before_tokens,
            compacted_tokens: after_tokens,
            message: format!("压缩完成：{} → {} tokens", before_tokens, after_tokens),
            before_char_count: before_chars,
            after_char_count: after_chars,
            before_token_count: before_tokens,
            after_token_count: after_tokens,
            boundary_message: boundary,
            is_auto,
        })
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
}
