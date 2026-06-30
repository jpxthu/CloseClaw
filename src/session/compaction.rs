//! Compaction module wrapper.
//!
//! Re-exports closeclaw-session compaction types and provides the
//! LLM-calling `execute_compact` function that bridges session types
//! to the actual LLM client.

use closeclaw_llm::fallback::FallbackClient;
use closeclaw_llm::{ChatRequest, Message};
use closeclaw_session::compaction::{
    build_compact_prompt, estimate_messages_tokens, estimate_tokens, extract_summary,
    format_boundary_message, CompactionError, CompactionMessage, CompactionResult,
};

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
    );

    // Filter: only user/assistant messages enter the compaction request.
    // System-prompt and other roles are excluded per compact-process.md.
    let conversation_messages: Vec<&Message> = messages
        .iter()
        .filter(|m| m.role == "user" || m.role == "assistant")
        .collect();

    // Build system prompt with compaction instructions
    let compact_prompt = build_compact_prompt(custom_instructions);
    let system_message = Message {
        role: "system".to_string(),
        content: compact_prompt,
    };

    let mut request_messages = vec![system_message];
    for msg in &conversation_messages {
        request_messages.push(Message {
            role: msg.role.clone(),
            content: msg.content.clone(),
        });
    }

    let request = ChatRequest {
        model: model_name.to_string(),
        messages: request_messages,
        temperature: 0.3,
        max_tokens: Some(8192),
    };

    let response = llm
        .chat(request)
        .await
        .map_err(|e| CompactionError::LLMCallFailed(e.to_string()))?;

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
