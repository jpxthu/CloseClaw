//! LLM integration for the active-searcher.
//!
//! Provides concept extraction and LLM-based event summarisation
//! via a mockable [`LlmCaller`] trait.

use async_trait::async_trait;

use closeclaw_llm::session::SessionMessage;
use closeclaw_llm::types::ContentBlock;

use super::active_searcher::ActiveSearcherError;

// ── LLM caller trait ─────────────────────────────────────────────────────

/// Abstract LLM caller for concept extraction and summarisation.
///
/// Implementors must be `Send + Sync` for use in async contexts.
/// In production, wrap a real [`Provider`][closeclaw_llm::Provider];
/// in tests, provide a mock that returns fixed strings.
#[async_trait]
pub trait LlmCaller: Send + Sync {
    /// Send a prompt to the LLM and return the text completion.
    async fn complete(&self, prompt: &str) -> Result<String, ActiveSearcherError>;
}

// ── Prompt builders ──────────────────────────────────────────────────────

/// Build the concept-extraction prompt.
///
/// Format:
/// ```text
/// System: Extract key concepts...
/// Context: [recent turns]
/// Message: [current message]
/// ```
pub(crate) fn build_concept_extraction_prompt(
    messages: &[SessionMessage],
    current_message: &str,
) -> String {
    let mut ctx = String::new();
    for msg in messages {
        let role = &msg.role;
        let text: String = msg
            .content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                ContentBlock::ToolResult { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        if !text.is_empty() {
            ctx.push_str(&format!("{}: {}\n", role, text));
        }
    }

    format!(
        "You are a concept extraction assistant. \
         Extract the key concepts from the conversation that would be useful \
         for searching a memory database. Cover three dimensions:\
         1. Action types — operations, tasks, or activities being performed.\
         2. Entities/objects — people, tools, files, or resources involved.\
         3. Scenario characteristics — context themes, settings, or conditions.\
         Avoid generic words like 'the', 'and', 'is'.\n\n\
         Context:\n{ctx}\n\
         Current message: {current_message}\n\n\
         Return a JSON array of concept strings. \
         Example: [\"debugging session\", \"Alice\", \"memory-miner\", \"late night\"]\n\
         Return ONLY the JSON array, nothing else."
    )
}

/// Build the event-summarisation prompt.
///
/// The LLM receives the raw event text and must condense it into a
/// concise summary within `max_chars` characters.
pub(crate) fn build_summarization_prompt(event_text: &str, max_chars: usize) -> String {
    format!(
        "Condense the following memory events into a concise summary. \
         Keep the most important information: who was involved, what happened, \
         and key details. Maximum {max_chars} characters.\n\n\
         Events:\n{event_text}"
    )
}

// ── Response parsers ─────────────────────────────────────────────────────

/// Parse the LLM response as a JSON array of concept strings.
///
/// Tolerant: extracts anything inside `[` and `]`, handling both
/// `"concept"` and `"concept",` formats.
pub(crate) fn parse_concepts(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();

    // Try standard JSON parse first.
    if let Ok(arr) = serde_json::from_str::<Vec<String>>(trimmed) {
        return arr;
    }

    // Fallback: find the JSON array substring.
    if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.find(']')) {
        let slice = &trimmed[start..=end];
        if let Ok(arr) = serde_json::from_str::<Vec<String>>(slice) {
            return arr;
        }
    }

    Vec::new()
}

// ── LLM pipeline methods ────────────────────────────────────────────────

/// Extract concepts via LLM from the current message and context.
///
/// Builds the prompt, calls the LLM, and parses the JSON array.
pub(crate) async fn extract_concepts_llm(
    caller: &dyn LlmCaller,
    context_messages: &[SessionMessage],
    current_message: &str,
) -> Result<Vec<String>, ActiveSearcherError> {
    let prompt = build_concept_extraction_prompt(context_messages, current_message);
    let raw = caller.complete(&prompt).await?;
    Ok(parse_concepts(&raw))
}

/// Summarise events via LLM.
///
/// Builds a prompt with the event text, calls the LLM, and returns
/// the condensed summary (truncated to `max_chars` as a safety net).
pub(crate) async fn summarize_events_llm(
    caller: &dyn LlmCaller,
    events_text: &str,
    max_chars: usize,
) -> Result<String, ActiveSearcherError> {
    let prompt = build_summarization_prompt(events_text, max_chars);
    let raw = caller.complete(&prompt).await?;
    // Safety truncation — LLM should respect the limit, but guard
    // against overflow. Use char-based truncation to avoid splitting
    // multi-byte UTF-8 characters.
    if raw.len() > max_chars {
        let mut end = 0;
        for (idx, ch) in raw.char_indices() {
            if idx + ch.len_utf8() > max_chars {
                break;
            }
            end = idx + ch.len_utf8();
        }
        Ok(raw[..end].to_string())
    } else {
        Ok(raw)
    }
}

// ── Role exclusion ───────────────────────────────────────────────────────

/// Session roles that should NOT trigger the active-searcher.
const EXCLUDED_ROLES: &[&str] = &["memory-miner", "dreaming"];

/// Returns `true` if the given session role should trigger active-searcher.
///
/// `memory-miner` and `dreaming` sessions are excluded to avoid
/// circular memory writes.
pub(crate) fn should_trigger_role(session_role: &str) -> bool {
    !EXCLUDED_ROLES.contains(&session_role)
}
