//! Tests for `ConversationSession::invoke_llm`.

use std::sync::Arc;

use crate::session::{ConversationSession, InjectionPosition, MemoryInjection};
use async_trait::async_trait;
use closeclaw_common::llm_types::InternalRequest;
use closeclaw_common::processor::UnifiedResponse;
use closeclaw_common::{LLMError, LlmCaller};

use super::tmp_path;

/// A fake LlmCaller that returns a canned response.
struct FakeLlmCaller {
    response: UnifiedResponse,
}

#[async_trait]
impl LlmCaller for FakeLlmCaller {
    async fn call(&self, _request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        Ok(self.response.clone())
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
        Err(LLMError::InvalidRequest(
            "streaming not supported in test".into(),
        ))
    }
}

/// A fake LlmCaller that always errors.
struct ErrorLlmCaller;

#[async_trait]
impl LlmCaller for ErrorLlmCaller {
    async fn call(&self, _request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        Err(LLMError::ApiError("simulated failure".into()))
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
        Err(LLMError::ApiError("not implemented".into()))
    }
}

fn canned_response(text: &str) -> UnifiedResponse {
    use closeclaw_common::processor::{ContentBlock, UnifiedUsage};
    UnifiedResponse {
        content_blocks: vec![ContentBlock::Text(text.into())],
        usage: UnifiedUsage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: Some(2),
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: Some("stop".into()),
    }
}

// ── invoke_llm error when no caller ────────────────────────────────────

#[tokio::test]
async fn test_invoke_llm_no_caller_returns_error() {
    let session = ConversationSession::new("s1".into(), "gpt-4o".into(), tmp_path());
    let result: Result<UnifiedResponse, LLMError> = session.invoke_llm("hello").await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        LLMError::InvalidRequest(msg) => {
            assert!(msg.contains("no LlmCaller"));
        }
        other => panic!("expected InvalidRequest, got {:?}", other),
    }
}

// ── invoke_llm success path ────────────────────────────────────────────

#[tokio::test]
async fn test_invoke_llm_success() {
    let mut session = ConversationSession::new("s2".into(), "gpt-4o".into(), tmp_path());
    let caller: Arc<dyn LlmCaller> = Arc::new(FakeLlmCaller {
        response: canned_response("Hi from LLM!"),
    });
    session.set_llm_caller(caller);

    let result: Result<UnifiedResponse, LLMError> = session.invoke_llm("hello").await;
    assert!(result.is_ok());
    let resp = result.unwrap();
    assert_eq!(resp.content_blocks.len(), 1);
    match &resp.content_blocks[0] {
        closeclaw_common::processor::ContentBlock::Text(t) => {
            assert_eq!(t, "Hi from LLM!");
        }
        other => panic!("expected Text block, got {:?}", other),
    }
}

// ── invoke_llm delegates error from caller ──────────────────────────────

#[tokio::test]
async fn test_invoke_llm_caller_error_propagates() {
    let mut session = ConversationSession::new("s3".into(), "gpt-4o".into(), tmp_path());
    let caller: Arc<dyn LlmCaller> = Arc::new(ErrorLlmCaller);
    session.set_llm_caller(caller);

    let result: Result<UnifiedResponse, LLMError> = session.invoke_llm("hello").await;
    assert!(result.is_err());
}

// ── invoke_llm consumes memory_injection (AfterCurrent) ─────────────────

#[tokio::test]
async fn test_invoke_llm_consumes_memory_injection_after_current() {
    let mut session = ConversationSession::new("s4".into(), "gpt-4o".into(), tmp_path());
    let caller: Arc<dyn LlmCaller> = Arc::new(FakeLlmCaller {
        response: canned_response("ok"),
    });
    session.set_llm_caller(caller);

    // Inject memory with AfterCurrent position
    let injection = MemoryInjection::new("context data".into(), InjectionPosition::AfterCurrent);
    session.set_memory_injection(injection);

    let result: Result<UnifiedResponse, LLMError> = session.invoke_llm("hello").await;
    assert!(result.is_ok());

    // Injection should have been consumed
    assert!(session.take_memory_injection().is_none());
}

// ── invoke_llm consumes memory_injection (BeforeNext) ───────────────────

#[tokio::test]
async fn test_invoke_llm_consumes_memory_injection_before_next() {
    let mut session = ConversationSession::new("s5".into(), "gpt-4o".into(), tmp_path());
    let caller: Arc<dyn LlmCaller> = Arc::new(FakeLlmCaller {
        response: canned_response("ok"),
    });
    session.set_llm_caller(caller);

    let injection = MemoryInjection::new("pre-context".into(), InjectionPosition::BeforeNext);
    session.set_memory_injection(injection);

    let result: Result<UnifiedResponse, LLMError> = session.invoke_llm("hello").await;
    assert!(result.is_ok());
    assert!(session.take_memory_injection().is_none());
}

// ── invoke_llm without memory_injection works fine ──────────────────────

#[tokio::test]
async fn test_invoke_llm_no_memory_injection() {
    let mut session = ConversationSession::new("s6".into(), "gpt-4o".into(), tmp_path());
    let caller: Arc<dyn LlmCaller> = Arc::new(FakeLlmCaller {
        response: canned_response("ok"),
    });
    session.set_llm_caller(caller);

    let result: Result<UnifiedResponse, LLMError> = session.invoke_llm("hello").await;
    assert!(result.is_ok());
}

// ── set_llm_caller / llm_caller getter ──────────────────────────────────

#[test]
fn test_set_and_get_llm_caller() {
    let mut session = ConversationSession::new("s7".into(), "gpt-4o".into(), tmp_path());
    assert!(session.llm_caller().is_none());

    let caller: Arc<dyn LlmCaller> = Arc::new(FakeLlmCaller {
        response: canned_response("x"),
    });
    session.set_llm_caller(caller.clone());
    assert!(session.llm_caller().is_some());

    // Verify the caller is the same Arc
    let got = session.llm_caller().unwrap();
    assert!(Arc::ptr_eq(got, &caller));
}
