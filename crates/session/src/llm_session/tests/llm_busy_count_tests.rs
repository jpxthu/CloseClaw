//! Tests for LLM busy-count tracking in `invoke_llm` and `invoke_llm_streaming`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::llm_types::InternalRequest;
use closeclaw_common::processor::{ContentBlockType, StreamEvent, UnifiedResponse, UnifiedUsage};
use closeclaw_common::{LLMError, LlmCaller, ShutdownSignal};
use futures::StreamExt;

use crate::llm_session::streaming_assembly::SessionStream;
use crate::llm_session::ConversationSession;

use super::tmp_path;

/// Minimal mock implementing `ShutdownSignal` for busy-count tests.
struct BusyCountMock {
    busy_count: AtomicUsize,
    shutting_down: bool,
    forceful: std::sync::atomic::AtomicBool,
}

impl BusyCountMock {
    fn new() -> Self {
        Self {
            busy_count: AtomicUsize::new(0),
            shutting_down: false,
            forceful: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn busy_count(&self) -> usize {
        self.busy_count.load(Ordering::SeqCst)
    }
}

impl ShutdownSignal for BusyCountMock {
    fn is_shutting_down(&self) -> bool {
        self.shutting_down
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
        self.forceful.store(true, Ordering::SeqCst);
        true
    }
    fn is_forceful(&self) -> bool {
        self.forceful.load(Ordering::SeqCst)
    }
    fn drain_status(&self) -> closeclaw_common::DrainStatus {
        closeclaw_common::DrainStatus {
            state: closeclaw_common::shutdown::ShutdownState::Running,
            busy_count: self.busy_count(),
            is_draining: false,
            pending_items: Vec::new(),
        }
    }
}

fn canned_usage() -> UnifiedUsage {
    UnifiedUsage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: Some(15),
        reasoning_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
    }
}

fn canned_response() -> UnifiedResponse {
    UnifiedResponse {
        content_blocks: vec![closeclaw_common::ContentBlock::Text("ok".into())],
        usage: canned_usage(),
        finish_reason: Some("stop".into()),
        retry_attempts: 0,
    }
}

/// A fake LlmCaller that returns a canned response and stream.
struct FakeCaller;

#[async_trait]
impl LlmCaller for FakeCaller {
    async fn call(&self, _request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        Ok(canned_response())
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
                delta: closeclaw_common::processor::ContentDelta::Text { text: "hi".into() },
            }),
            Ok(StreamEvent::BlockEnd {
                index: 0,
                block_type: ContentBlockType::Text,
            }),
            Ok(StreamEvent::MessageEnd {
                usage: Some(canned_usage()),
                finish_reason: Some("stop".into()),
            }),
        ];
        Ok(Box::pin(stream::iter(events)))
    }
}

/// A fake LlmCaller that fails on `call_streaming`.
struct FailingStreamCaller;

#[async_trait]
impl LlmCaller for FailingStreamCaller {
    async fn call(&self, _request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        Ok(canned_response())
    }

    async fn call_streaming(
        &self,
        _request: InternalRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamEvent, LLMError>> + Send>>,
        LLMError,
    > {
        Err(LLMError::ApiError("stream init failed".into()))
    }
}

/// A fake LlmCaller that produces a stream with an error event.
struct ErrorInStreamCaller;

#[async_trait]
impl LlmCaller for ErrorInStreamCaller {
    async fn call(&self, _request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        Ok(canned_response())
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
            Err(LLMError::ApiError("mid-stream error".into())),
        ];
        Ok(Box::pin(stream::iter(events)))
    }
}

// ── invoke_llm busy-count ─────────────────────────────────────────────

#[tokio::test]
async fn test_invoke_llm_success_increments_and_decrements_busy() {
    let mut session = ConversationSession::new("s1".into(), "gpt-4o".into(), tmp_path());
    let sh = Arc::new(BusyCountMock::new());
    session.set_shutdown_handle(sh.clone() as Arc<dyn ShutdownSignal>);

    let caller: Arc<dyn LlmCaller> = Arc::new(FakeCaller);
    session.set_llm_caller(caller);

    assert_eq!(sh.busy_count(), 0);
    let result = session.invoke_llm("hello").await;
    assert!(result.is_ok());
    assert_eq!(
        sh.busy_count(),
        0,
        "busy_count must return to 0 after LLM call"
    );
}

#[tokio::test]
async fn test_invoke_llm_error_still_decrements_busy() {
    let mut session = ConversationSession::new("s2".into(), "gpt-4o".into(), tmp_path());
    let sh = Arc::new(BusyCountMock::new());
    session.set_shutdown_handle(sh.clone() as Arc<dyn ShutdownSignal>);

    // No LLM caller set → returns InvalidRequest error
    assert_eq!(sh.busy_count(), 0);
    let result = session.invoke_llm("hello").await;
    assert!(result.is_err());
    assert_eq!(
        sh.busy_count(),
        0,
        "busy_count must return to 0 even when LLM call errors"
    );
}

// ── invoke_llm_streaming busy-count ───────────────────────────────────

#[tokio::test]
async fn test_invoke_llm_streaming_success_decrements_on_stream_end() {
    let mut session = ConversationSession::new("s3".into(), "gpt-4o".into(), tmp_path());
    let sh = Arc::new(BusyCountMock::new());
    session.set_shutdown_handle(sh.clone() as Arc<dyn ShutdownSignal>);

    let caller: Arc<dyn LlmCaller> = Arc::new(FakeCaller);
    session.set_llm_caller(caller);

    assert_eq!(sh.busy_count(), 0);
    let stream_result = session.invoke_llm_streaming("hello").await;
    assert!(stream_result.is_ok());
    // busy_count should be 1 (incremented before stream creation, not yet decremented)
    assert_eq!(
        sh.busy_count(),
        1,
        "busy_count should be 1 before stream is consumed"
    );

    let mut stream = stream_result.unwrap();
    // Consume the stream to completion.
    while let Some(_item) = stream.next().await {}
    assert_eq!(
        sh.busy_count(),
        0,
        "busy_count must return to 0 after stream is fully consumed"
    );
}

#[tokio::test]
async fn test_invoke_llm_streaming_caller_error_decrements_immediately() {
    let mut session = ConversationSession::new("s4".into(), "gpt-4o".into(), tmp_path());
    let sh = Arc::new(BusyCountMock::new());
    session.set_shutdown_handle(sh.clone() as Arc<dyn ShutdownSignal>);

    let caller: Arc<dyn LlmCaller> = Arc::new(FailingStreamCaller);
    session.set_llm_caller(caller);

    assert_eq!(sh.busy_count(), 0);
    let result = session.invoke_llm_streaming("hello").await;
    assert!(result.is_err());
    assert_eq!(
        sh.busy_count(),
        0,
        "busy_count must return to 0 when call_streaming fails"
    );
}

#[tokio::test]
async fn test_invoke_llm_streaming_mid_stream_error_decrements() {
    let mut session = ConversationSession::new("s5".into(), "gpt-4o".into(), tmp_path());
    let sh = Arc::new(BusyCountMock::new());
    session.set_shutdown_handle(sh.clone() as Arc<dyn ShutdownSignal>);

    let caller: Arc<dyn LlmCaller> = Arc::new(ErrorInStreamCaller);
    session.set_llm_caller(caller);

    assert_eq!(sh.busy_count(), 0);
    let stream_result = session.invoke_llm_streaming("hello").await;
    assert!(stream_result.is_ok());
    assert_eq!(sh.busy_count(), 1);

    let mut stream = stream_result.unwrap();
    // Consume stream — should encounter the error.
    while let Some(item) = stream.next().await {
        if item.is_err() {
            break;
        }
    }
    assert_eq!(
        sh.busy_count(),
        0,
        "busy_count must return to 0 after stream error"
    );
}

// ── SessionStream shutdown handle integration ──────────────────────────

#[tokio::test]
async fn test_session_stream_decrements_on_normal_end() {
    let sh = Arc::new(BusyCountMock::new());
    sh.increment_busy();
    assert_eq!(sh.busy_count(), 1);

    let events = vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
    ];

    let inner: std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<StreamEvent, LLMError>> + Send>,
    > = Box::pin(futures::stream::iter(events));

    let stream = SessionStream::new(inner).with_shutdown_handle(sh.clone());
    let mut pinned = Box::pin(stream);

    while let Some(_item) = pinned.next().await {}
    assert_eq!(
        sh.busy_count(),
        0,
        "SessionStream must decrement busy on normal end"
    );
}

#[tokio::test]
async fn test_session_stream_decrements_on_error() {
    let sh = Arc::new(BusyCountMock::new());
    sh.increment_busy();
    assert_eq!(sh.busy_count(), 1);

    let events: Vec<Result<StreamEvent, LLMError>> = vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Err(LLMError::ApiError("boom".into())),
    ];

    let inner: std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<StreamEvent, LLMError>> + Send>,
    > = Box::pin(futures::stream::iter(events));

    let stream = SessionStream::new(inner).with_shutdown_handle(sh.clone());
    let mut pinned = Box::pin(stream);

    // Consume until error.
    while let Some(item) = pinned.next().await {
        if item.is_err() {
            break;
        }
    }
    assert_eq!(
        sh.busy_count(),
        0,
        "SessionStream must decrement busy on stream error"
    );
}

#[tokio::test]
async fn test_session_stream_without_handle_does_not_panic() {
    let events = vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
    ];

    let inner: std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<StreamEvent, LLMError>> + Send>,
    > = Box::pin(futures::stream::iter(events));

    // No shutdown handle attached — should work fine.
    let stream = SessionStream::new(inner);
    let mut pinned = Box::pin(stream);

    let mut count = 0;
    while let Some(_item) = pinned.next().await {
        count += 1;
    }
    assert!(count > 0, "stream should yield events");
}

// ── Streaming forceful upgrade ────────────────────────────────────────

/// When `escalate_to_forceful` is called during streaming, the stream
/// must terminate early with `Err(LLMError::Cancelled)` on the next poll.
#[tokio::test]
async fn test_streaming_forceful_upgrade_terminates_stream() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct ForcefulMock {
        busy_count: AtomicUsize,
        forceful: AtomicBool,
    }

    impl ForcefulMock {
        fn new() -> Self {
            Self {
                busy_count: AtomicUsize::new(0),
                forceful: AtomicBool::new(false),
            }
        }
    }

    impl ShutdownSignal for ForcefulMock {
        fn is_shutting_down(&self) -> bool {
            false
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
            self.forceful.store(true, Ordering::SeqCst);
            true
        }
        fn is_forceful(&self) -> bool {
            self.forceful.load(Ordering::SeqCst)
        }
        fn drain_status(&self) -> closeclaw_common::DrainStatus {
            closeclaw_common::DrainStatus {
                state: closeclaw_common::shutdown::ShutdownState::Running,
                busy_count: self.busy_count(),
                is_draining: false,
                pending_items: Vec::new(),
            }
        }
    }

    let sh = Arc::new(ForcefulMock::new());
    sh.increment_busy();
    assert_eq!(sh.busy_count(), 1);

    let events = vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::processor::ContentDelta::Text {
                text: "before forceful".into(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::processor::ContentDelta::Text {
                text: "after forceful".into(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
    ];

    let inner: std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<StreamEvent, LLMError>> + Send>,
    > = Box::pin(futures::stream::iter(events));

    let stream = SessionStream::new(inner).with_shutdown_handle(sh.clone());
    let mut pinned = Box::pin(stream);

    // Consume first two events (BlockStart + first BlockDelta).
    let first = pinned.next().await.unwrap();
    assert!(first.is_ok(), "first event should be Ok");
    let second = pinned.next().await.unwrap();
    assert!(second.is_ok(), "second event should be Ok");

    // Now escalate to forceful — next poll should detect and terminate.
    sh.escalate_to_forceful();
    assert!(sh.is_forceful());

    let third = pinned.next().await;
    match third {
        Some(Err(LLMError::Cancelled)) => {}
        other => panic!(
            "expected Some(Err(Cancelled)) after forceful upgrade, got {:?}",
            other
        ),
    }

    // Stream should be finished; no more events.
    assert!(
        pinned.next().await.is_none(),
        "stream should be finished after forceful termination"
    );

    // Busy count must have been decremented.
    assert_eq!(
        sh.busy_count(),
        0,
        "busy_count must return to 0 after forceful stream termination"
    );
}

/// Forceful upgrade on an idle stream (no events yet) terminates immediately.
#[tokio::test]
async fn test_streaming_forceful_upgrade_idle_stream() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct ForcefulMock2 {
        busy_count: AtomicUsize,
        forceful: AtomicBool,
    }

    impl ForcefulMock2 {
        fn new() -> Self {
            Self {
                busy_count: AtomicUsize::new(0),
                forceful: AtomicBool::new(false),
            }
        }
    }

    impl ShutdownSignal for ForcefulMock2 {
        fn is_shutting_down(&self) -> bool {
            false
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
            self.forceful.store(true, Ordering::SeqCst);
            true
        }
        fn is_forceful(&self) -> bool {
            self.forceful.load(Ordering::SeqCst)
        }
        fn drain_status(&self) -> closeclaw_common::DrainStatus {
            closeclaw_common::DrainStatus {
                state: closeclaw_common::shutdown::ShutdownState::Running,
                busy_count: self.busy_count(),
                is_draining: false,
                pending_items: Vec::new(),
            }
        }
    }

    let sh = Arc::new(ForcefulMock2::new());
    sh.increment_busy();

    // Use a stream that never yields — simulates pending LLM response.
    let inner: std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<StreamEvent, LLMError>> + Send>,
    > = Box::pin(futures::stream::pending());

    let stream = SessionStream::new(inner).with_shutdown_handle(sh.clone());
    let mut pinned = Box::pin(stream);

    // Escalate immediately before any poll.
    sh.escalate_to_forceful();

    // First poll should detect forceful and terminate.
    let result = pinned.next().await.unwrap();
    match result {
        Err(LLMError::Cancelled) => {}
        other => panic!(
            "expected Cancelled on idle stream with forceful upgrade, got {:?}",
            other
        ),
    }

    assert_eq!(sh.busy_count(), 0);
}
