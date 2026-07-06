//! Tests for `ConversationSession::invoke_llm` and `invoke_llm_streaming`.

use std::sync::Arc;

use crate::llm_session::{ConversationSession, InjectionPosition, MemoryInjection};
use async_trait::async_trait;
use closeclaw_common::llm_types::InternalRequest;
use closeclaw_common::processor::UnifiedResponse;
use closeclaw_common::{LLMError, LlmCaller};

use super::tmp_path;

/// A fake LlmCaller that returns a canned response and supports streaming.
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
        use closeclaw_common::processor::{ContentBlockType, StreamEvent};
        use futures::stream;

        // Produce a minimal valid stream: BlockStart → BlockDelta → BlockEnd → MessageEnd
        let text = self
            .response
            .content_blocks
            .iter()
            .filter_map(|b| match b {
                closeclaw_common::processor::ContentBlock::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        let events: Vec<Result<StreamEvent, LLMError>> = vec![
            Ok(StreamEvent::BlockStart {
                index: 0,
                block_type: ContentBlockType::Text,
            }),
            Ok(StreamEvent::BlockDelta {
                index: 0,
                delta: closeclaw_common::processor::ContentDelta::Text { text },
            }),
            Ok(StreamEvent::BlockEnd {
                index: 0,
                block_type: ContentBlockType::Text,
            }),
            Ok(StreamEvent::MessageEnd {
                usage: Some(self.response.usage.clone()),
                finish_reason: self.response.finish_reason.clone(),
            }),
        ];
        Ok(Box::pin(stream::iter(events)))
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

// ── invoke_llm_streaming error when no caller ────────────────────────

#[tokio::test]
async fn test_invoke_llm_streaming_no_caller_returns_error() {
    let session = ConversationSession::new("s_stream_1".into(), "gpt-4o".into(), tmp_path());
    let result = session.invoke_llm_streaming("hello").await;
    assert!(result.is_err(), "expected error when no LlmCaller injected");
}

// ── invoke_llm_streaming success path ────────────────────────────────

#[tokio::test]
async fn test_invoke_llm_streaming_success() {
    use futures::StreamExt;

    let mut session = ConversationSession::new("s_stream_2".into(), "gpt-4o".into(), tmp_path());
    let caller: Arc<dyn LlmCaller> = Arc::new(FakeLlmCaller {
        response: canned_response("streamed"),
    });
    session.set_llm_caller(caller);

    let result = session.invoke_llm_streaming("hello").await;
    assert!(result.is_ok());

    // Collect all events from the stream
    let mut stream = result.unwrap();
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event);
    }
    assert!(!events.is_empty());
}

// ── invoke_llm_streaming consumes memory_injection ────────────────────

#[tokio::test]
async fn test_invoke_llm_streaming_consumes_memory_injection() {
    let mut session = ConversationSession::new("s_stream_3".into(), "gpt-4o".into(), tmp_path());
    let caller: Arc<dyn LlmCaller> = Arc::new(FakeLlmCaller {
        response: canned_response("ok"),
    });
    session.set_llm_caller(caller);

    let injection = MemoryInjection::new("stream context".into(), InjectionPosition::AfterCurrent);
    session.set_memory_injection(injection);
    assert!(session.take_memory_injection().is_some());

    // Set it again for the streaming call
    let injection2 = MemoryInjection::new("stream context 2".into(), InjectionPosition::BeforeNext);
    session.set_memory_injection(injection2);

    let result = session.invoke_llm_streaming("hello").await;
    assert!(result.is_ok());

    // Injection should have been consumed by the streaming call
    assert!(session.take_memory_injection().is_none());
}

// ── set_prompt_overrides setter ──────────────────────────────────────

#[test]
fn test_set_prompt_overrides() {
    use closeclaw_common::PromptOverrides;

    let mut session = ConversationSession::new("s_overrides".into(), "gpt-4o".into(), tmp_path());

    // Default: no overrides
    assert!(!session.has_system_prompt_builder());

    // Set overrides
    session.set_prompt_overrides(Some(PromptOverrides {
        override_prompt: Some("custom".to_string()),
        agent_prompt: None,
        custom_prompt: None,
    }));

    // Rebuild should pick up overrides via the session's stored field
    // (tested indirectly via rebuild_system_prompt_tests)
}

// ── memory_injection_arc access ──────────────────────────────────────

#[test]
fn test_memory_injection_arc_provides_access() {
    let session = ConversationSession::new("s_arc".into(), "gpt-4o".into(), tmp_path());
    let arc = session.memory_injection_arc();

    // Initially empty
    {
        let slot = arc.lock().unwrap();
        assert!(slot.is_none());
    }

    // Set via session method, read via arc
    session.set_memory_injection(MemoryInjection::new(
        "via_arc".into(),
        InjectionPosition::AfterCurrent,
    ));
    {
        let slot = arc.lock().unwrap();
        assert!(slot.is_some());
        assert_eq!(slot.as_ref().unwrap().content, "via_arc");
    }
}

// ── Gateway delegation: dispatch_llm_call delegates to invoke_llm ────

/// Verify the delegation path: `ConversationSession::invoke_llm` is the
/// single entry point for LLM calls. This test confirms the session
/// layer correctly delegates to the injected caller, which is the same
/// path used by `SessionMessageHandler::dispatch_llm_call` in Gateway.
#[tokio::test]
async fn test_session_delegates_to_injected_caller() {
    use closeclaw_common::processor::ContentBlock;

    let mut session = ConversationSession::new("s_delegate".into(), "gpt-4o".into(), tmp_path());
    let caller: Arc<dyn LlmCaller> = Arc::new(FakeLlmCaller {
        response: UnifiedResponse {
            content_blocks: vec![ContentBlock::Text("delegated response".into())],
            usage: closeclaw_common::processor::UnifiedUsage {
                prompt_tokens: 5,
                completion_tokens: 3,
                total_tokens: Some(8),
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: Some("stop".into()),
        },
    });
    session.set_llm_caller(caller);

    let result = session.invoke_llm("test delegation").await.unwrap();
    match &result.content_blocks[0] {
        ContentBlock::Text(t) => assert_eq!(t, "delegated response"),
        other => panic!("expected Text, got {:?}", other),
    }
    assert_eq!(result.usage.total_tokens, Some(8));
}
