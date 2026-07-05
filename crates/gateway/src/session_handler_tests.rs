use super::*;
use crate::session_handler::ActiveSearcherLlmCaller;
use closeclaw_common::LlmCaller;
use closeclaw_llm::fallback::FallbackClient;
use closeclaw_llm::retry::CooldownManager;
use closeclaw_llm::session_state::LlmState;
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;
use closeclaw_llm::LLMRegistry;
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::persistence::ReasoningLevel;

/// Create a `SessionMessageHandler` with a mock LLM caller injected
/// into the `SessionManager`. Must be called BEFORE `find_or_create`
/// so the `ConversationSession` gets the caller at creation time.
async fn handler_with_sm(sm: Arc<SessionManager>) -> SessionMessageHandler {
    let registry = Arc::new(LLMRegistry::new());
    let fallback = Arc::new(FallbackClient::from_strings(registry, vec![]));
    let ufc = Arc::new(UnifiedFallbackClient::new(
        vec![],
        Arc::new(CooldownManager::new()),
    ));
    let llm_caller: Arc<dyn LlmCaller> = Arc::new(llm_caller_impl::FallbackLlmCaller(ufc.clone()));
    // Set LLM caller on SessionManager so ConversationSession gets it at creation.
    sm.set_llm_caller(llm_caller).await;
    let fallback_llm_caller = Arc::new(ActiveSearcherLlmCaller {
        client: ufc,
        model: String::new(),
    });
    SessionMessageHandler::new_no_output(sm, fallback, fallback_llm_caller)
}

fn make_msg() -> crate::Message {
    use std::collections::HashMap;
    crate::Message {
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

fn make_config() -> crate::GatewayConfig {
    crate::GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: crate::DmScope::default(),
        ..Default::default()
    }
}

fn make_sm() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        &make_config(),
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ))
}

#[tokio::test]
async fn test_idle_message_returns_llm_started() {
    let sm = make_sm();
    // Inject LLM caller BEFORE creating sessions
    let handler = handler_with_sm(Arc::clone(&sm)).await;
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();
    let result = handler.handle_message(&sid, "hello".to_string()).await;
    assert!(matches!(result, HandleResult::LlmStarted));
}

#[tokio::test]
async fn test_busy_message_returns_queued() {
    let sm = make_sm();
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    // Manually set busy
    if let Some(cs) = sm.get_conversation_session(&sid).await {
        cs.write().await.set_llm_busy(true);
        cs.write().await.set_llm_state(LlmState::Requesting);
    }

    let handler = handler_with_sm(Arc::clone(&sm)).await;
    let result = handler.handle_message(&sid, "hello".to_string()).await;
    assert!(matches!(result, HandleResult::MessageQueued));

    // Verify message was actually enqueued
    if let Some(pending) = sm.pop_pending_message(&sid).await {
        assert_eq!(pending.content, "hello");
    } else {
        panic!("expected pending message");
    }
}

#[tokio::test]
async fn test_no_pending_no_recursion() {
    let sm = make_sm();
    let handler = handler_with_sm(Arc::clone(&sm)).await;
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    // With empty fallback chain, call will fail — but we just verify it doesn't panic
    handler.handle_message(&sid, "hello".to_string()).await;
    // Give the task a moment to run
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // No pending messages exist
    assert!(sm.pop_pending_message(&sid).await.is_none());
}

/// After an LLM call completes (even with empty chain → failure),
/// busy should be cleared so the session becomes idle again.
#[tokio::test]
async fn test_llm_failure_resets_busy() {
    let sm = make_sm();
    let handler = handler_with_sm(Arc::clone(&sm)).await;
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    // Start a call — busy becomes true
    let result = handler.handle_message(&sid, "hello".to_string()).await;
    assert!(matches!(result, HandleResult::LlmStarted));
    assert!(
        sm.is_session_busy(&sid).await,
        "busy should be true immediately after call"
    );

    // Wait for the async LLM task to finish (it will fail because chain is empty)
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Busy should be cleared after LLM failure
    assert!(
        !sm.is_session_busy(&sid).await,
        "busy should be reset to false after LLM failure"
    );
}

/// After an LLM call completes, pending messages are automatically drained
/// and the session handles them in order.
#[tokio::test]
async fn test_pending_consumed_after_llm_done() {
    let sm = make_sm();
    let handler = handler_with_sm(Arc::clone(&sm)).await;
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    // First message starts LLM call, busy = true
    handler.handle_message(&sid, "first".to_string()).await;
    assert!(sm.is_session_busy(&sid).await);

    // Second message while busy → enqueued
    let result = handler.handle_message(&sid, "second".to_string()).await;
    assert!(matches!(result, HandleResult::MessageQueued));

    // Wait for first LLM call to finish and drain pending
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    // After drain: no more pending (the "second" message was consumed by drain loop)
    assert!(
        sm.pop_pending_message(&sid).await.is_none(),
        "pending message should have been consumed during drain"
    );
}

/// Multiple pending messages are consumed in FIFO order.
#[tokio::test]
async fn test_multiple_pending_fifo_order() {
    let sm = make_sm();
    let handler = handler_with_sm(Arc::clone(&sm)).await;
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    // Start first LLM call
    handler.handle_message(&sid, "first".to_string()).await;

    // Enqueue two more while busy
    handler.handle_message(&sid, "second".to_string()).await;
    handler.handle_message(&sid, "third".to_string()).await;

    // Verify order by draining all pending and checking order
    let mut pending = Vec::new();
    while let Some(msg) = sm.pop_pending_message(&sid).await {
        pending.push(msg);
    }
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].content, "second");
    assert_eq!(pending[1].content, "third");

    // Wait for all LLM calls to finish (first + two drained)
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // All pending should have been drained
    assert!(sm.pop_pending_message(&sid).await.is_none());
}

// `/compact` tests removed — /compact is now handled by the SlashDispatcher
// at the Gateway level, not by SessionMessageHandler. See slash_permission tests.

// `/clear` tests removed — /clear is now handled by the SlashDispatcher
// at the Gateway level, not by SessionMessageHandler. See slash_permission tests.

/// Verifying that setting verbosity level on a ConversationSession persists
/// across multiple accesses via `get_conversation_session`.
#[tokio::test]
async fn test_set_verbosity_persists() {
    use closeclaw_common::VerbosityLevel;

    let sm = make_sm();
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    // Verify default verbosity is Full
    let cs = sm.get_conversation_session(&sid).await.expect("session");
    assert_eq!(cs.read().await.verbosity_level(), VerbosityLevel::Full);

    // Set verbosity to Normal
    cs.write().await.set_verbosity_level(VerbosityLevel::Normal);

    // Drop the read/write guard and re-acquire to verify persistence
    drop(cs);
    let cs2 = sm.get_conversation_session(&sid).await.expect("session");
    assert_eq!(cs2.read().await.verbosity_level(), VerbosityLevel::Normal);

    // Set verbosity to Off
    cs2.write().await.set_verbosity_level(VerbosityLevel::Off);
    drop(cs2);

    // Verify Off persists
    let cs3 = sm.get_conversation_session(&sid).await.expect("session");
    assert_eq!(cs3.read().await.verbosity_level(), VerbosityLevel::Off);
}
