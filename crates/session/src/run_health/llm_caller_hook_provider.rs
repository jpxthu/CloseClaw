//! Production [`HookLlmProvider`] backed by an [`LlmCaller`].
//!
//! Wraps the session-level `LlmCaller` trait to implement the lightweight
//! LLM review calls used by hook reviewers. Converts the `review` call
//! into a non-streaming `LlmCaller::call` with a simple prompt.

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::{InternalMessage, InternalRequest, LlmCaller};

use super::hook_reviewer::HookLlmProvider;

/// Production implementation of [`HookLlmProvider`] backed by an
/// [`LlmCaller`].
///
/// Constructs a simple single-message `InternalRequest` from the
/// prompt and context, sends it via non-streaming `call`, and
/// parses the response text for a boolean answer.
pub struct LlmCallerHookProvider {
    caller: Arc<dyn LlmCaller>,
}

impl LlmCallerHookProvider {
    /// Create a new provider wrapping the given `LlmCaller`.
    pub fn new(caller: Arc<dyn LlmCaller>) -> Self {
        Self { caller }
    }
}

#[async_trait]
impl HookLlmProvider for LlmCallerHookProvider {
    async fn review(&self, prompt: &str, context: &str) -> Result<bool, String> {
        let content = format!("{prompt}\n\n{context}");

        let request = InternalRequest {
            model: String::new(),
            messages: vec![InternalMessage {
                role: "user".to_string(),
                content,
                tool_call_id: None,
            }],
            temperature: 0.0,
            max_tokens: Some(64),
            stream: false,
            extra_body: Default::default(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            tools: None,
            session_id: None,
            reasoning_level: closeclaw_common::ReasoningLevel::default(),
            turn_count: None,
        };

        match self.caller.call(request).await {
            Ok(response) => {
                let text = extract_text(&response);
                Ok(is_affirmative(&text))
            }
            Err(_) => Ok(false),
        }
    }
}

/// Extract concatenated text from all `Text` content blocks in a response.
fn extract_text(response: &closeclaw_common::processor::UnifiedResponse) -> String {
    response
        .content_blocks
        .iter()
        .filter_map(|block| match block {
            closeclaw_common::processor::ContentBlock::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Check if the response text is affirmative.
///
/// Matches "yes", "true", "是" (case-insensitive for English).
fn is_affirmative(text: &str) -> bool {
    let lower = text.to_lowercase();
    let trimmed = lower.trim();
    trimmed.contains("yes") || trimmed.contains("true") || trimmed.contains("是")
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_common::processor::{ContentBlock, UnifiedResponse, UnifiedUsage};

    fn make_response(text: &str) -> UnifiedResponse {
        UnifiedResponse {
            content_blocks: vec![ContentBlock::Text(text.to_string())],
            usage: UnifiedUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: None,
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: Some("stop".to_string()),
            retry_attempts: 0,
        }
    }

    #[test]
    fn test_is_affirmative_yes() {
        assert!(is_affirmative("YES"));
        assert!(is_affirmative("yes"));
        assert!(is_affirmative("Yes"));
        assert!(is_affirmative("  yes  "));
    }

    #[test]
    fn test_is_affirmative_true() {
        assert!(is_affirmative("true"));
        assert!(is_affirmative("TRUE"));
        assert!(is_affirmative("True"));
    }

    #[test]
    fn test_is_affirmative_chinese() {
        assert!(is_affirmative("是"));
        assert!(is_affirmative("是的"));
    }

    #[test]
    fn test_is_not_affirmative() {
        assert!(!is_affirmative("no"));
        assert!(!is_affirmative("false"));
        assert!(!is_affirmative("否"));
        assert!(!is_affirmative(""));
    }

    #[test]
    fn test_extract_text() {
        let response = make_response("hello world");
        assert_eq!(extract_text(&response), "hello world");
    }
}
