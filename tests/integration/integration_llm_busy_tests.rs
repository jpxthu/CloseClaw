//! Integration tests for SessionMessageHandler busy/pending state machine.
//!
//! Verifies that:
//! 1. LLM busy → new messages are queued, not processed
//! 2. LLM idle → pending messages are consumed in FIFO order
//! 3. FakeProvider call count per concurrent period never exceeds 1
//!
//! The LLM caller is injected at the session level (via
//! `SessionManager::set_llm_caller` + `FallbackLlmCaller` wrapping a
//! `UnifiedFallbackClient`), matching the current Gateway architecture.
//!
//! Uses `#[cfg(feature = "fake-llm")]` to gate all tests on the feature flag.

#![cfg(feature = "fake-llm")]

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use closeclaw_common::llm_caller::LlmCaller;
use closeclaw_gateway::llm_caller_impl::FallbackLlmCaller;
use closeclaw_gateway::session_handler::{
    ActiveSearcherLlmCaller, HandleResult, SessionMessageHandler,
};
use closeclaw_gateway::session_manager::SessionManager;
use closeclaw_gateway::{GatewayConfig, Message};
use closeclaw_llm::client::UnifiedChatClient;
use closeclaw_llm::fake::{FakeProvider, Scenario};
use closeclaw_llm::fallback::FallbackClient;
use closeclaw_llm::interpreter::InterpreterRegistry;
use closeclaw_llm::plugin::PluginPipeline;
use closeclaw_llm::protocol::OpenAiProtocol;
use closeclaw_llm::retry::CooldownManager;
use closeclaw_llm::session_state::LlmState;
use closeclaw_llm::unified_fallback::{ChainEntry, UnifiedFallbackClient};
use closeclaw_session::persistence::ReasoningLevel;

/// Build a `SessionMessageHandler` whose session-level LLM caller wraps the
/// given `FakeProvider` through the full unified-fallback stack
/// (`FallbackLlmCaller` → `UnifiedFallbackClient` → `UnifiedChatClient`).
///
/// This must be called BEFORE `find_or_create` so that `resolve` injects the
/// caller into every newly-created `ConversationSession`.
async fn build_handler(sm: Arc<SessionManager>, provider: FakeProvider) -> SessionMessageHandler {
    let cooldown = Arc::new(CooldownManager::new());

    let client = Arc::new(UnifiedChatClient::with_noop_cache_adapter(
        Arc::new(provider),
        Arc::new(OpenAiProtocol::default()),
        InterpreterRegistry::new(vec![]),
        PluginPipeline::new(),
    ));
    let entry = ChainEntry {
        provider_id: "fake".to_string(),
        model_id: "fake-model".to_string(),
        client,
    };
    let unified = Arc::new(UnifiedFallbackClient::new(vec![entry], cooldown));

    let llm_caller: Arc<dyn LlmCaller> = Arc::new(FallbackLlmCaller(unified.clone()));
    // Set the session-level LLM caller BEFORE find_or_create.
    sm.set_llm_caller(llm_caller).await;

    // The legacy FallbackClient is still held by the handler for the
    // compaction path; these tests never trigger compaction, so an empty
    // chain is sufficient.
    let registry = Arc::new(closeclaw_llm::LLMRegistry::new());
    let fallback = Arc::new(FallbackClient::from_strings(registry, vec![]));

    let fallback_llm_caller = Arc::new(ActiveSearcherLlmCaller {
        client: unified,
        model: "fake-model".to_string(),
    });

    SessionMessageHandler::new_no_output(
        sm,
        fallback,
        fallback_llm_caller,
        closeclaw_session::compaction::CompactConfig::default(),
    )
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
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    }
}

/// Assert that a pending queue is empty for a session.
async fn assert_no_pending(sm: &SessionManager, sid: &str) {
    assert!(
        sm.pop_pending_message(sid).await.is_none(),
        "expected no pending messages for session {sid}"
    );
}

/// Number of currently queued pending messages for a session (read-only).
async fn pending_count(sm: &SessionManager, sid: &str) -> usize {
    match sm.get_conversation_session(sid).await {
        Some(cs) => cs.read().await.get_pending_messages().len(),
        None => 0,
    }
}

/// Poll `cond` until it returns `true` or `timeout` elapses, yielding briefly
/// between checks. Bounded readiness polling: it asks "is the drain done?"
/// instead of "has a fixed sleep duration elapsed?".
async fn wait_until<F, Fut>(timeout: Duration, mut cond: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if cond().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
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

    let provider = FakeProvider::builder()
        .then_delay(
            Duration::from_millis(200),
            Scenario::ok("response-1", "fake-model"),
        )
        .then_ok("response-2", "fake-model")
        .build();
    let provider_ref = provider.clone();

    let handler = build_handler(sm.clone(), provider).await;
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    let result = handler.handle_message(&sid, "first".to_string()).await;
    assert!(matches!(result, HandleResult::LlmStarted));
    assert!(sm.is_session_busy(&sid).await, "session should be busy");

    // Yield to let the spawned task start the LLM call
    tokio::time::sleep(Duration::from_millis(20)).await;
    // Only one call started so far
    assert_eq!(provider_ref.captured_internal_requests().len(), 1);
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

    let provider = FakeProvider::builder()
        .then_ok("response", "fake-model")
        .build();

    let handler = build_handler(sm.clone(), provider).await;
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    // Manually set busy (like SessionMessageHandler does)
    let cs = sm.get_conversation_session(&sid).await.unwrap();
    cs.write().await.set_llm_busy(true);
    cs.write().await.set_llm_state(LlmState::Requesting);

    let result = handler.handle_message(&sid, "hello".to_string()).await;
    assert!(matches!(result, HandleResult::MessageQueued(_)));

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

    let provider = FakeProvider::builder()
        .then_delay(
            Duration::from_millis(300),
            Scenario::ok("response-1", "fake-model"),
        )
        .then_ok("response-2", "fake-model")
        .or_else("fallback-ok")
        .build();
    let provider_ref = provider.clone();

    let handler = build_handler(sm.clone(), provider).await;
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    // First message — starts LLM call, busy = true
    let result1 = handler.handle_message(&sid, "first".to_string()).await;
    assert!(matches!(result1, HandleResult::LlmStarted));
    assert!(sm.is_session_busy(&sid).await);

    // Yield to let the spawned task start the LLM call
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Immediately send second message — should be queued
    let result2 = handler.handle_message(&sid, "second".to_string()).await;
    assert!(matches!(result2, HandleResult::MessageQueued(_)));

    // FakeProvider should have received only 1 call so far
    assert_eq!(
        provider_ref.captured_internal_requests().len(),
        1,
        "only one LLM call should have been made while busy"
    );

    // Wait for the drain to finish (bounded readiness polling, not a blind
    // sleep): session idle, both LLM calls recorded, pending queue empty.
    let drained = wait_until(Duration::from_secs(5), || async {
        !sm.is_session_busy(&sid).await
            && provider_ref.captured_internal_requests().len() == 2
            && pending_count(&sm, &sid).await == 0
    })
    .await;
    assert!(
        drained,
        "drain did not finish within 5s: busy={}, calls={}, pending={}",
        sm.is_session_busy(&sid).await,
        provider_ref.captured_internal_requests().len(),
        pending_count(&sm, &sid).await
    );
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

    let provider = FakeProvider::builder()
        .then_ok("response-1", "fake-model")
        .then_ok("response-2", "fake-model")
        .then_ok("response-3", "fake-model")
        .then_ok("response-4", "fake-model")
        .or_else("fallback")
        .build();
    let provider_ref = provider.clone();

    let handler = build_handler(sm.clone(), provider).await;
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    // Manually mark the session busy so the following messages queue
    // deterministically (a real in-flight LLM call holds the session
    // write lock for its whole duration, which makes the busy check racy).
    {
        let cs = sm.get_conversation_session(&sid).await.unwrap();
        cs.write().await.set_llm_busy(true);
        cs.write().await.set_llm_state(LlmState::Requesting);
    }

    // Messages arriving while busy → queued.
    let result1 = handler.handle_message(&sid, "first".to_string()).await;
    let result2 = handler.handle_message(&sid, "second".to_string()).await;
    let result3 = handler.handle_message(&sid, "third".to_string()).await;
    assert!(matches!(result1, HandleResult::MessageQueued(_)));
    assert!(matches!(result2, HandleResult::MessageQueued(_)));
    assert!(matches!(result3, HandleResult::MessageQueued(_)));

    // Verify FIFO order in the queue.
    let m1 = sm.pop_pending_message(&sid).await.unwrap();
    let m2 = sm.pop_pending_message(&sid).await.unwrap();
    let m3 = sm.pop_pending_message(&sid).await.unwrap();
    assert_eq!(m1.content, "first");
    assert_eq!(m2.content, "second");
    assert_eq!(m3.content, "third");

    // Re-queue them since the drain loop consumes from the same queue.
    sm.push_pending_message(&sid, m1).await.unwrap();
    sm.push_pending_message(&sid, m2).await.unwrap();
    sm.push_pending_message(&sid, m3).await.unwrap();

    // Clear busy so the next message dispatches and the drain runs.
    {
        let cs = sm.get_conversation_session(&sid).await.unwrap();
        cs.write().await.set_llm_busy(false);
        cs.write().await.set_llm_state(LlmState::Idle);
    }

    // Dispatch a real message; after its LLM call completes, the drain
    // loop processes the queued messages in FIFO order.
    let trigger = handler.handle_message(&sid, "trigger".to_string()).await;
    assert!(matches!(trigger, HandleResult::LlmStarted));

    // Wait for the trigger + drain to process all queued messages (bounded
    // readiness polling, not a blind sleep).
    let drained = wait_until(Duration::from_secs(5), || async {
        !sm.is_session_busy(&sid).await
            && provider_ref.captured_internal_requests().len() == 4
            && pending_count(&sm, &sid).await == 0
    })
    .await;
    assert!(
        drained,
        "drain did not finish within 5s: busy={}, calls={}, pending={}",
        sm.is_session_busy(&sid).await,
        provider_ref.captured_internal_requests().len(),
        pending_count(&sm, &sid).await
    );

    // All 4 calls (trigger + 3 queued) should have been made, in FIFO order.
    let contents: Vec<String> = provider_ref
        .captured_internal_requests()
        .iter()
        .map(|c| {
            c.request
                .messages
                .iter()
                .rev()
                .find(|m| m.role == "user")
                .map(|m| m.content.clone())
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(
        contents,
        vec![
            "trigger".to_string(),
            "first".to_string(),
            "second".to_string(),
            "third".to_string()
        ],
        "pending messages should be consumed in FIFO order"
    );
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

    let provider = FakeProvider::builder()
        .then_delay(
            Duration::from_millis(200),
            Scenario::ok("response-1", "fake-model"),
        )
        .then_ok("response-2", "fake-model")
        .or_else("fallback")
        .build();

    let handler = build_handler(sm.clone(), provider).await;
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    // Start first call
    handler.handle_message(&sid, "first".to_string()).await;
    assert!(sm.is_session_busy(&sid).await);

    // Queue a second message
    handler.handle_message(&sid, "second".to_string()).await;

    // Wait for the drain to finish (idle + no pending) with bounded
    // readiness polling, not a blind fixed-duration sleep.
    let drained = wait_until(Duration::from_secs(5), || async {
        !sm.is_session_busy(&sid).await && pending_count(&sm, &sid).await == 0
    })
    .await;
    assert!(
        drained,
        "drain did not finish within 5s: busy={}, pending={}",
        sm.is_session_busy(&sid).await,
        pending_count(&sm, &sid).await
    );
    assert_no_pending(&sm, &sid).await;
}
