//! Compaction execution bridge.
//!
//! Provides the [`execute_compact`] function that bridges session
//! compaction types with the LLM client. Lives in the `llm` crate
//! because it needs [`FallbackClient`] while the session crate
//! intentionally avoids depending on `llm`.

pub use closeclaw_session::compaction::{
    build_compact_prompt, estimate_messages_tokens, estimate_tokens, extract_summary,
    format_boundary_message, get_context_window, CompactConfig, CompactionError, CompactionMessage,
    CompactionResult, CompactionService, TokenWarningState, BASE_COMPACT_PROMPT,
    MODEL_CONTEXT_WINDOWS, NO_TOOLS_PREAMBLE, NO_TOOLS_TRAILER,
};

use crate::fallback::FallbackClient;
use crate::{ChatRequest, Message};

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
    llm: &FallbackClient,
    model_name: &str,
    custom_instructions: Option<&str>,
    is_auto: bool,
    chars_per_token: f64,
) -> Result<CompactionResult, CompactionError> {
    if messages.is_empty() {
        return Err(CompactionError::EmptyMessages);
    }

    let before_char_count: usize = messages.iter().map(|m| m.content.chars().count()).sum();
    let before_token_count = estimate_messages_tokens(
        &messages
            .iter()
            .map(|m| CompactionMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect::<Vec<_>>(),
        chars_per_token,
    );

    // Filter: only user/assistant messages enter the compaction request.
    // System-prompt and other roles are excluded per compact-process.md.
    let conversation_messages: Vec<&Message> = messages
        .iter()
        .filter(|m| m.role == "user" || m.role == "assistant")
        .collect();

    // Build system prompt with compaction instructions
    let compact_prompt = build_compact_prompt(custom_instructions);

    // Build LLM request with system prompt + filtered conversation
    let mut llm_messages = vec![Message {
        role: "system".to_string(),
        content: compact_prompt,
    }];

    for msg in conversation_messages {
        llm_messages.push(Message {
            role: msg.role.clone(),
            content: msg.content.clone(),
        });
    }

    let request = ChatRequest {
        model: model_name.to_string(),
        messages: llm_messages,
        temperature: 0.0,
        max_tokens: Some(4096),
    };

    let response = llm
        .chat(request)
        .await
        .map_err(|e| CompactionError::LLMCallFailed(e.to_string()))?;

    let summary = extract_summary(&response.content).ok_or(CompactionError::SummaryParseFailed)?;

    let boundary = format_boundary_message(&summary, is_auto, chrono::Utc::now());
    let after_token_count = estimate_tokens(&boundary, chars_per_token);
    let after_char_count = boundary.chars().count();

    Ok(CompactionResult {
        performed: true,
        original_tokens: before_token_count,
        compacted_tokens: after_token_count,
        message: format!(
            "Compaction completed: {} → {} tokens",
            before_token_count, after_token_count
        ),
        before_char_count,
        after_char_count,
        before_token_count,
        after_token_count,
        boundary_message: boundary,
        is_auto,
    })
}
