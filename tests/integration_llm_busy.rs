//! Integration tests for SessionMessageHandler busy/pending state machine.
//!
//! Verifies that:
//! 1. LLM busy → new messages are queued, not processed
//! 2. LLM idle → pending messages are consumed in FIFO order
//! 3. FakeProvider call count per concurrent period never exceeds 1
//!
//! Uses `#[cfg(feature = "fake-llm")]` to gate all tests on the feature flag.

#![cfg(feature = "fake-llm")]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use closeclaw_gateway::session_handler::{HandleResult, SessionMessageHandler};
use closeclaw_gateway::session_manager::SessionManager;
use closeclaw_gateway::{GatewayConfig, Message};
use closeclaw_llm::client::UnifiedChatClient;
use closeclaw_llm::fake::{FakeProvider, Scenario};
use closeclaw_llm::fallback::{FallbackClient, ModelEntry};
use closeclaw_llm::protocol::{ChatProtocol, IncomingSseStream, OutgoingEventStream};
use closeclaw_llm::provider::Provider;
use closeclaw_llm::types::{InternalRequest, InternalResponse, ProtocolId, SseStateMachine};
use closeclaw_llm::LLMRegistry;
use closeclaw_session::persistence::ReasoningLevel;
use reqwest::header::HeaderMap;

// ---------------------------------------------------------------------------
// Minimal stub types for UnifiedChatClient construction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct StubProtocol {
    id: ProtocolId,
}

#[async_trait]
impl ChatProtocol for StubProtocol {
    fn protocol_id(&self) -> &ProtocolId {
        &self.id
    }
    fn path(&self) -> &str {
        "/chat"
    }
    fn build_request(
        &self,
        _req: &InternalRequest,
    ) -> closeclaw_llm::protocol::Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }
    fn parse_response(
        &self,
        _body: serde_json::Value,
    ) -> closeclaw_llm::protocol::Result<InternalResponse> {
        unimplemented!("stub protocol")
    }
    fn decorate_headers(&self, _headers: &mut HeaderMap) -> closeclaw_llm::protocol::Result<()> {
        Ok(())
    }
    fn create_sse_machine(&self) -> SseStateMachine {
        unimplemented!("stub protocol")
    }
    async fn parse_sse_stream(
        &self,
        _incoming: IncomingSseStream,
        _machine: SseStateMachine,
    ) -> OutgoingEventStream {
        unimplemented!("stub protocol")
    }
}

/// Wrap a FakeProvider into `Arc<dyn Provider>`.
fn wrap_provider(provider: FakeProvider) -> Arc<dyn Provider> {
    Arc::new(provider)
}

/// Build a `UnifiedChatClient` from a wrapped provider.
fn make_unified_client(provider: Arc<dyn Provider>) -> Arc<UnifiedChatClient> {
    Arc::new(UnifiedChatClient::with_noop_cache_adapter(
        provider,
        Arc::new(StubProtocol {
            id: ProtocolId::from("stub"),
        }),
        Default::default(),
        Default::default(),
    ))
}

/// Create a minimal GatewayConfig for testing.
fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

/// Create a dummy gateway Message for find_or_create.
fn make_msg() -> Message {
    use std::collections::HashMap;
    Message {
        id: "msg_1".into(),
        from: "alice".into(),
        to: "bob".into(),
        content: "hello".into(),
        channel: "ch".into(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
        thread_id: None,
    }
}

/// Assert that a pending queue is empty for a session.
async fn assert_no_pending(sm: &SessionManager, sid: &str) {
    assert!(
        sm.pop_pending_message(sid).await.is_none(),
        "expected no pending messages for session {sid}"
    );
}

/// The first message sent to an idle session should return LlmStarted.
#[tokio::test]
async fn test_idle_message_returns_llm_started() {
    let sm = Arc::new(SessionManager::new(
        &test_config(),
        None,
        None,
        ReasoningLevel::default(),
    ));
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    let provider = FakeProvider::builder()
        .then_delay(
            Duration::from_millis(200),
            Scenario::ok("response-1", "fake-model"),
        )
        .then_ok("response-2", "fake-model")
        .build();
    let provider_ref = provider.clone();

    let registry = Arc::new(LLMRegistry::new());
    registry
        .register("fake".to_string(), wrap_provider(provider))
        .await;

    let fallback = Arc::new(FallbackClient::new(
        registry,
        vec![ModelEntry {
            provider: "fake".to_string(),
            model: "fake-model".to_string(),
        }],
    ));
    let handler = SessionMessageHandler::new_no_output(
        sm.clone(),
        fallback,
        make_unified_client(wrap_provider(
            FakeProvider::builder()
                .then_ok("dummy", "fake-model")
                .build(),
        )),
    );

    let result = handler.handle_message(&sid, "first".to_string()).await;
    assert!(matches!(result, HandleResult::LlmStarted));
    assert!(sm.is_session_busy(&sid).await, "session should be busy");

    // Yield to let the spawned task start the LLM call
    tokio::time::sleep(Duration::from_millis(20)).await;
    // Only one call started so far
    assert_eq!(provider_ref.captured_requests().len(), 1);
}

/// A message sent while the LLM is busy should be queued.
#[tokio::test]
async fn test_busy_message_returns_queued() {
    let sm = Arc::new(SessionManager::new(
        &test_config(),
        None,
        None,
        ReasoningLevel::default(),
    ));
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    // Manually set busy (like SessionMessageHandler does)
    let cs = sm.get_conversation_session(&sid).await.unwrap();
    cs.write().await.set_llm_busy(true);

    let provider = FakeProvider::builder()
        .then_ok("response", "fake-model")
        .build();
    let registry = Arc::new(LLMRegistry::new());
    registry
        .register("fake".to_string(), wrap_provider(provider))
        .await;

    let fallback = Arc::new(FallbackClient::new(
        registry,
        vec![ModelEntry {
            provider: "fake".to_string(),
            model: "fake-model".to_string(),
        }],
    ));
    let handler = SessionMessageHandler::new_no_output(
        sm.clone(),
        fallback,
        make_unified_client(wrap_provider(
            FakeProvider::builder()
                .then_ok("dummy", "fake-model")
                .build(),
        )),
    );

    let result = handler.handle_message(&sid, "hello".to_string()).await;
    assert!(matches!(result, HandleResult::MessageQueued));

    // Verify the message was actually enqueued
    let pending = sm.pop_pending_message(&sid).await;
    assert!(pending.is_some(), "expected a pending message");
    assert_eq!(pending.unwrap().content, "hello");
}

/// When the LLM is busy and a new message arrives:
/// - The new message is queued (MessageQueued returned)
/// - FakeProvider should only have received 1 call (the first one)
#[tokio::test]
async fn test_fake_provider_call_count_while_busy() {
    let sm = Arc::new(SessionManager::new(
        &test_config(),
        None,
        None,
        ReasoningLevel::default(),
    ));
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    let provider = FakeProvider::builder()
        .then_delay(
            Duration::from_millis(300),
            Scenario::ok("response-1", "fake-model"),
        )
        .then_ok("response-2", "fake-model")
        .or_else("fallback-ok")
        .build();
    let provider_ref = provider.clone();

    let registry = Arc::new(LLMRegistry::new());
    registry
        .register("fake".to_string(), wrap_provider(provider))
        .await;

    let fallback = Arc::new(FallbackClient::new(
        registry,
        vec![ModelEntry {
            provider: "fake".to_string(),
            model: "fake-model".to_string(),
        }],
    ));
    let handler = SessionMessageHandler::new_no_output(
        sm.clone(),
        fallback,
        make_unified_client(wrap_provider(
            FakeProvider::builder()
                .then_ok("dummy", "fake-model")
                .build(),
        )),
    );

    // First message — starts LLM call, busy = true
    let result1 = handler.handle_message(&sid, "first".to_string()).await;
    assert!(matches!(result1, HandleResult::LlmStarted));
    assert!(sm.is_session_busy(&sid).await);

    // Yield to let the spawned task start the LLM call
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Immediately send second message — should be queued
    let result2 = handler.handle_message(&sid, "second".to_string()).await;
    assert!(matches!(result2, HandleResult::MessageQueued));

    // FakeProvider should have received only 1 call so far
    assert_eq!(
        provider_ref.captured_requests().len(),
        1,
        "only one LLM call should have been made while busy"
    );

    // Wait for delay (300ms) + drain to complete
    tokio::time::sleep(Duration::from_millis(700)).await;

    // After drain: session should be idle, both calls made, no pending
    assert!(
        !sm.is_session_busy(&sid).await,
        "session should be idle after drain"
    );
    assert_eq!(
        provider_ref.captured_requests().len(),
        2,
        "both LLM calls should have been made after drain"
    );
    assert_no_pending(&sm, &sid).await;
}

/// Pending messages are consumed in FIFO order after the LLM completes.
#[tokio::test]
async fn test_pending_fifo_after_delay() {
    let sm = Arc::new(SessionManager::new(
        &test_config(),
        None,
        None,
        ReasoningLevel::default(),
    ));
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    let provider = FakeProvider::builder()
        .then_delay(
            Duration::from_millis(200),
            Scenario::ok("response-1", "fake-model"),
        )
        .then_ok("response-2", "fake-model")
        .then_ok("response-3", "fake-model")
        .or_else("fallback")
        .build();
    let provider_ref = provider.clone();

    let registry = Arc::new(LLMRegistry::new());
    registry
        .register("fake".to_string(), wrap_provider(provider))
        .await;

    let fallback = Arc::new(FallbackClient::new(
        registry,
        vec![ModelEntry {
            provider: "fake".to_string(),
            model: "fake-model".to_string(),
        }],
    ));
    let handler = SessionMessageHandler::new_no_output(
        sm.clone(),
        fallback,
        make_unified_client(wrap_provider(
            FakeProvider::builder()
                .then_ok("dummy", "fake-model")
                .build(),
        )),
    );

    // First message → LlmStarted (busy)
    handler.handle_message(&sid, "first".to_string()).await;
    // Yield to let spawned task start
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(provider_ref.captured_requests().len(), 1);

    // Two more messages while busy → both queued
    handler.handle_message(&sid, "second".to_string()).await;
    handler.handle_message(&sid, "third".to_string()).await;

    // Verify queue order
    let m2 = sm.pop_pending_message(&sid).await.unwrap();
    assert_eq!(m2.content, "second");
    let m3 = sm.pop_pending_message(&sid).await.unwrap();
    assert_eq!(m3.content, "third");
    // Re-queue them since drain_pending_loop consumes from the same queue
    sm.push_pending_message(&sid, m2).await.unwrap();
    sm.push_pending_message(&sid, m3).await.unwrap();
    assert_eq!(provider_ref.captured_requests().len(), 1);

    // Wait for delay + drain to process all
    tokio::time::sleep(Duration::from_millis(800)).await;

    // All 3 calls should have been made
    assert_eq!(provider_ref.captured_requests().len(), 3);
    assert!(!sm.is_session_busy(&sid).await);
    assert_no_pending(&sm, &sid).await;
}

/// After the LLM finishes and pending messages are drained,
/// the session should be idle with no pending messages.
#[tokio::test]
async fn test_idle_after_delay_drain() {
    let sm = Arc::new(SessionManager::new(
        &test_config(),
        None,
        None,
        ReasoningLevel::default(),
    ));
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    let provider = FakeProvider::builder()
        .then_delay(
            Duration::from_millis(200),
            Scenario::ok("response-1", "fake-model"),
        )
        .then_ok("response-2", "fake-model")
        .or_else("fallback")
        .build();

    let registry = Arc::new(LLMRegistry::new());
    registry
        .register("fake".to_string(), wrap_provider(provider))
        .await;

    let fallback = Arc::new(FallbackClient::new(
        registry,
        vec![ModelEntry {
            provider: "fake".to_string(),
            model: "fake-model".to_string(),
        }],
    ));
    let handler = SessionMessageHandler::new_no_output(
        sm.clone(),
        fallback,
        make_unified_client(wrap_provider(
            FakeProvider::builder()
                .then_ok("dummy", "fake-model")
                .build(),
        )),
    );

    // Start first call
    handler.handle_message(&sid, "first".to_string()).await;
    assert!(sm.is_session_busy(&sid).await);

    // Queue a second message
    handler.handle_message(&sid, "second".to_string()).await;

    // Wait long enough for delay + both calls to complete
    tokio::time::sleep(Duration::from_millis(800)).await;

    // Session should be idle
    assert!(!sm.is_session_busy(&sid).await, "session should be idle");
    assert_no_pending(&sm, &sid).await;
}
