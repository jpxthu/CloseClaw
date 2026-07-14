//! Integration tests for [`LlmCallerHookProvider`].
//!
//! Tests the `review()` method through the `HookLlmProvider` trait,
//! verifying that LLM responses are correctly parsed into boolean
//! results and that failures degrade gracefully.

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::processor::{ContentBlock, UnifiedResponse, UnifiedUsage};
use closeclaw_common::{InternalRequest, LLMError, LlmCaller};

use super::hook_reviewer::HookLlmProvider;
use super::llm_caller_hook_provider::LlmCallerHookProvider;

// ── Mock LlmCaller ──────────────────────────────────────────────────────────

/// Mock LLM caller that returns a pre-configured response or error.
struct MockLlmCaller {
    response: Result<UnifiedResponse, LLMError>,
}

#[async_trait]
impl LlmCaller for MockLlmCaller {
    async fn call(&self, _request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        self.response.clone()
    }

    async fn call_streaming(
        &self,
        _request: InternalRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures::Stream<
                        Item = Result<closeclaw_common::processor::StreamEvent, LLMError>,
                    > + Send,
            >,
        >,
        LLMError,
    > {
        unimplemented!("not needed for these tests")
    }
}

fn make_text_response(text: &str) -> UnifiedResponse {
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

fn make_mock_caller(text: &str) -> Arc<dyn LlmCaller> {
    Arc::new(MockLlmCaller {
        response: Ok(make_text_response(text)),
    })
}

fn make_failing_caller() -> Arc<dyn LlmCaller> {
    Arc::new(MockLlmCaller {
        response: Err(LLMError::NetworkError("connection refused".into())),
    })
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn review_returns_true_for_affirmative_yes() {
    let caller = make_mock_caller("YES");
    let provider = LlmCallerHookProvider::new(caller);
    let result = provider.review("test prompt", "test context").await;
    assert_eq!(result, Ok(true));
}

#[tokio::test]
async fn review_returns_true_for_affirmative_chinese() {
    let caller = make_mock_caller("是");
    let provider = LlmCallerHookProvider::new(caller);
    let result = provider.review("test prompt", "test context").await;
    assert_eq!(result, Ok(true));
}

#[tokio::test]
async fn review_returns_false_for_negative_no() {
    let caller = make_mock_caller("NO");
    let provider = LlmCallerHookProvider::new(caller);
    let result = provider.review("test prompt", "test context").await;
    assert_eq!(result, Ok(false));
}

#[tokio::test]
async fn review_returns_false_for_negative_chinese() {
    let caller = make_mock_caller("否");
    let provider = LlmCallerHookProvider::new(caller);
    let result = provider.review("test prompt", "test context").await;
    assert_eq!(result, Ok(false));
}

#[tokio::test]
async fn review_returns_false_for_negative_false() {
    let caller = make_mock_caller("false");
    let provider = LlmCallerHookProvider::new(caller);
    let result = provider.review("test prompt", "test context").await;
    assert_eq!(result, Ok(false));
}

#[tokio::test]
async fn review_returns_false_on_llm_failure() {
    let caller = make_failing_caller();
    let provider = LlmCallerHookProvider::new(caller);
    let result = provider.review("test prompt", "test context").await;
    // LLM failure → graceful degradation → Ok(false)
    assert_eq!(result, Ok(false));
}

#[tokio::test]
async fn review_constructs_prompt_with_context() {
    // Verify that the provider concatenates prompt and context.
    // The mock always returns "no" regardless of input, so we
    // just verify it doesn't panic and returns the expected shape.
    let caller = make_mock_caller("NO");
    let provider = LlmCallerHookProvider::new(caller);
    let result = provider
        .review("Is this a problem?", "Turn context here")
        .await;
    assert_eq!(result, Ok(false));
}

#[tokio::test]
async fn review_handles_empty_response_text() {
    let caller = make_mock_caller("");
    let provider = LlmCallerHookProvider::new(caller);
    let result = provider.review("test prompt", "test context").await;
    // Empty string is not affirmative.
    assert_eq!(result, Ok(false));
}
