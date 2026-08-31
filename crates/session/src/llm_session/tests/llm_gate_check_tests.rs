//! Step 1.5 — LLM gate check tests for `invoke_llm` and `invoke_llm_streaming`.
//!
//! Verifies:
//! - Normal path: non-shutting-down sessions proceed with LLM call
//! - Shutdown rejection: `is_shutting_down()` returns Cancelled error
//! - No shutdown handle: behavior is unchanged

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::llm_types::InternalRequest;
use closeclaw_common::processor::{ContentBlockType, StreamEvent, UnifiedResponse, UnifiedUsage};
use closeclaw_common::{LLMError, LlmCaller, ShutdownSignal};

use crate::llm_session::ConversationSession;

use super::tmp_path;

/// Minimal mock implementing `ShutdownSignal` for gate check tests.
struct GateCheckMock {
    shutting_down: AtomicBool,
    busy_count: std::sync::atomic::AtomicUsize,
}

impl GateCheckMock {
    fn new() -> Self {
        Self {
            shutting_down: AtomicBool::new(false),
            busy_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn set_shutting_down(&self, v: bool) {
        self.shutting_down.store(v, Ordering::SeqCst);
    }
}

impl ShutdownSignal for GateCheckMock {
    fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }
    fn increment_busy(&self) {
        self.busy_count.fetch_add(1, Ordering::SeqCst);
    }
    fn decrement_busy(&self) {
        self.busy_count.fetch_sub(1, Ordering::SeqCst);
    }
    fn busy_count(&self) -> usize {
        self.busy_count.load(Ordering::SeqCst)
    }
    fn escalate_to_forceful(&self) -> bool {
        false
    }
    fn is_forceful(&self) -> bool {
        false
    }
    fn drain_status(&self) -> closeclaw_common::DrainStatus {
        closeclaw_common::DrainStatus {
            state: closeclaw_common::shutdown::ShutdownState::Running,
            busy_count: self.busy_count.load(Ordering::SeqCst),
            is_draining: false,
            pending_items: Vec::new(),
        }
    }
}

/// A fake LlmCaller that returns a canned response.
struct FakeCaller;

#[async_trait]
impl LlmCaller for FakeCaller {
    async fn call(&self, _request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        Ok(UnifiedResponse {
            content_blocks: vec![closeclaw_common::ContentBlock::Text("ok".into())],
            usage: UnifiedUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: Some(15),
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: Some("stop".into()),
            retry_attempts: 0,
        })
    }

    async fn call_streaming(
        &self,
        _request: InternalRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamEvent, LLMError>> + Send>>,
        LLMError,
    > {
        use futures::stream;
        let events = vec![
            Ok(StreamEvent::BlockStart {
                index: 0,
                block_type: ContentBlockType::Text,
            }),
            Ok(StreamEvent::BlockDelta {
                index: 0,
                delta: closeclaw_common::processor::ContentDelta::Text {
                    text: "streamed".into(),
                },
            }),
            Ok(StreamEvent::BlockEnd {
                index: 0,
                block_type: ContentBlockType::Text,
            }),
            Ok(StreamEvent::MessageEnd {
                usage: Some(UnifiedUsage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: Some(15),
                    reasoning_tokens: None,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                }),
                finish_reason: Some("stop".into()),
            }),
        ];
        Ok(Box::pin(stream::iter(events)))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Normal path: non-shutting-down sessions proceed with LLM call
// ═══════════════════════════════════════════════════════════════════════════

/// `invoke_llm` succeeds when `is_shutting_down()` is false.
#[tokio::test]
async fn test_invoke_llm_proceeds_when_not_shutting_down() {
    let mut session = ConversationSession::new("s_gate_normal".into(), "gpt-4o".into(), tmp_path());
    let sh = Arc::new(GateCheckMock::new());
    session.set_shutdown_handle(sh.clone() as Arc<dyn ShutdownSignal>);

    let caller: Arc<dyn LlmCaller> = Arc::new(FakeCaller);
    session.set_llm_caller(caller);

    // Not shutting down — call should succeed.
    let result = session.invoke_llm("hello").await;
    assert!(
        result.is_ok(),
        "invoke_llm should succeed when not shutting down"
    );
}

/// `invoke_llm_streaming` succeeds when `is_shutting_down()` is false.
#[tokio::test]
async fn test_invoke_llm_streaming_proceeds_when_not_shutting_down() {
    let mut session =
        ConversationSession::new("s_gate_stream_normal".into(), "gpt-4o".into(), tmp_path());
    let sh = Arc::new(GateCheckMock::new());
    session.set_shutdown_handle(sh.clone() as Arc<dyn ShutdownSignal>);

    let caller: Arc<dyn LlmCaller> = Arc::new(FakeCaller);
    session.set_llm_caller(caller);

    // Not shutting down — streaming call should succeed.
    let result = session.invoke_llm_streaming("hello").await;
    assert!(
        result.is_ok(),
        "invoke_llm_streaming should succeed when not shutting down"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Shutdown rejection: `is_shutting_down()` returns Cancelled error
// ═══════════════════════════════════════════════════════════════════════════

/// `invoke_llm` returns `Cancelled` when shutting down.
#[tokio::test]
async fn test_invoke_llm_returns_cancelled_when_shutting_down() {
    let mut session = ConversationSession::new("s_gate_reject".into(), "gpt-4o".into(), tmp_path());
    let sh = Arc::new(GateCheckMock::new());
    session.set_shutdown_handle(sh.clone() as Arc<dyn ShutdownSignal>);

    let caller: Arc<dyn LlmCaller> = Arc::new(FakeCaller);
    session.set_llm_caller(caller);

    sh.set_shutting_down(true);

    let result = session.invoke_llm("hello").await;
    assert!(result.is_err(), "invoke_llm should fail when shutting down");
    match result.unwrap_err() {
        LLMError::Cancelled => {}
        other => panic!("expected Cancelled, got {:?}", other),
    }
}

/// `invoke_llm_streaming` returns `Cancelled` when shutting down.
#[tokio::test]
async fn test_invoke_llm_streaming_returns_cancelled_when_shutting_down() {
    let mut session =
        ConversationSession::new("s_gate_stream_reject".into(), "gpt-4o".into(), tmp_path());
    let sh = Arc::new(GateCheckMock::new());
    session.set_shutdown_handle(sh.clone() as Arc<dyn ShutdownSignal>);

    let caller: Arc<dyn LlmCaller> = Arc::new(FakeCaller);
    session.set_llm_caller(caller);

    sh.set_shutting_down(true);

    let result = session.invoke_llm_streaming("hello").await;
    assert!(
        result.is_err(),
        "invoke_llm_streaming should fail when shutting down"
    );
    match result.err().unwrap() {
        LLMError::Cancelled => {}
        other => panic!("expected Cancelled, got {:?}", other),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// No shutdown handle: behavior is unchanged
// ═══════════════════════════════════════════════════════════════════════════

/// `invoke_llm` succeeds without any shutdown handle attached.
#[tokio::test]
async fn test_invoke_llm_succeeds_without_shutdown_handle() {
    let mut session =
        ConversationSession::new("s_gate_no_handle".into(), "gpt-4o".into(), tmp_path());
    // No shutdown handle set.

    let caller: Arc<dyn LlmCaller> = Arc::new(FakeCaller);
    session.set_llm_caller(caller);

    let result = session.invoke_llm("hello").await;
    assert!(
        result.is_ok(),
        "invoke_llm should succeed without shutdown handle"
    );
}

/// `invoke_llm_streaming` succeeds without any shutdown handle attached.
#[tokio::test]
async fn test_invoke_llm_streaming_succeeds_without_shutdown_handle() {
    let mut session = ConversationSession::new(
        "s_gate_stream_no_handle".into(),
        "gpt-4o".into(),
        tmp_path(),
    );
    // No shutdown handle set.

    let caller: Arc<dyn LlmCaller> = Arc::new(FakeCaller);
    session.set_llm_caller(caller);

    let result = session.invoke_llm_streaming("hello").await;
    assert!(
        result.is_ok(),
        "invoke_llm_streaming should succeed without shutdown handle"
    );
}
