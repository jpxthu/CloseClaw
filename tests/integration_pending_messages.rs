//! Integration tests for pending message state transitions in ConversationSession.
//!
//! Verifies:
//! 1. `push_pending()` queues a message with sent=false
//! 2. `mark_sent()` flips the sent flag to true
//! 3. `restore_pending_messages()` discards sent=true messages, retains sent=false ones
//!
//! Uses `#[cfg(feature = "fake-llm")]` to gate all tests, consistent with the rest of the
//! integration test suite.

#![cfg(feature = "fake-llm")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

use closeclaw_gateway::session_manager::SessionManager;
use closeclaw_gateway::{DmScope, GatewayConfig, Message};
use closeclaw_llm::fake::FakeProvider;
use closeclaw_llm::provider::Provider;
use closeclaw_llm::LLMRegistry;
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::persistence::PendingMessage;
use closeclaw_session::persistence::ReasoningLevel;

/// Build a minimal GatewayConfig for testing.
fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: DmScope::default(),
        ..Default::default()
    }
}

/// Build a dummy gateway Message for find_or_create.
fn make_msg() -> Message {
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

/// Set up a SessionManager with a FakeProvider registered.
/// Must be called from within a tokio runtime (e.g., inside a #[tokio::test]).
async fn setup_session_manager() -> Arc<SessionManager> {
    let sm = Arc::new(SessionManager::new(
        &test_config(),
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));

    let registry = Arc::new(LLMRegistry::new());
    let provider = FakeProvider::builder()
        .then_ok("fake response", "fake-model")
        .build();
    let wrapped: Arc<dyn Provider> = Arc::new(provider);
    registry.register("fake".to_string(), wrapped).await;

    sm
}

// ---------------------------------------------------------------------------
// Test 1: push_pending queues message with sent=false
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_push_pending_queue_non_empty_and_sent_false() {
    let sm = setup_session_manager().await;
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    // Push a pending message
    let msg = PendingMessage::new("msg-push-1".to_string(), "hello world".to_string());
    sm.push_pending_message(&sid, msg).await.unwrap();

    // Pop and verify
    let popped = sm.pop_pending_message(&sid).await;
    assert!(popped.is_some(), "queue should not be empty after push");

    let popped = popped.unwrap();
    assert_eq!(popped.message_id, "msg-push-1");
    assert_eq!(popped.content, "hello world");
    assert!(!popped.sent, "newly pushed message should have sent=false");
}

// ---------------------------------------------------------------------------
// Test 2: mark_sent flips the sent flag
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mark_sent_changes_flag() {
    let mut msg = PendingMessage::new("msg-mark-1".to_string(), "content".to_string());

    assert!(
        !msg.sent,
        "new message should have sent=false before mark_sent"
    );

    msg.mark_sent();

    assert!(msg.sent, "mark_sent() should set sent=true");
}

// ---------------------------------------------------------------------------
// Test 3: restore_pending_messages discards sent=true, keeps sent=false
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_restore_skips_sent_true() {
    use closeclaw_session::llm_session::ConversationSession;

    let test_root = TempDir::new().unwrap();
    let mut session = ConversationSession::new(
        "restore-test".to_string(),
        "fake-model".to_string(),
        test_root.path().to_path_buf(),
    );

    // Build a mixed list: one sent=true, one sent=false
    let mut msg_sent = PendingMessage::new("msg-sent".to_string(), "already sent".to_string());
    msg_sent.mark_sent();

    let msg_unsent = PendingMessage::new("msg-unsent".to_string(), "not yet sent".to_string());
    // sent=false by default

    let messages = vec![msg_sent, msg_unsent];
    session.restore_pending_messages(messages);

    // Only the unsent message should be restored
    let pending = session.get_pending_messages();
    assert_eq!(
        pending.len(),
        1,
        "only sent=false messages should be restored, got {} messages",
        pending.len()
    );
    assert_eq!(
        pending[0].message_id, "msg-unsent",
        "restored message should be the one with sent=false"
    );
}

// ---------------------------------------------------------------------------
// Extra: push two messages, pop in FIFO order
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_push_two_messages_fifo_order() {
    let sm = setup_session_manager().await;
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    let msg1 = PendingMessage::new("fifo-1".to_string(), "first".to_string());
    let msg2 = PendingMessage::new("fifo-2".to_string(), "second".to_string());

    sm.push_pending_message(&sid, msg1).await.unwrap();
    sm.push_pending_message(&sid, msg2).await.unwrap();

    let first = sm.pop_pending_message(&sid).await.unwrap();
    let second = sm.pop_pending_message(&sid).await.unwrap();

    assert_eq!(first.message_id, "fifo-1");
    assert_eq!(second.message_id, "fifo-2");
}

// ---------------------------------------------------------------------------
// Extra: queue is empty after draining all messages
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_queue_empty_after_drain() {
    let sm = setup_session_manager().await;
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    let msg = PendingMessage::new("drain-1".to_string(), "drain me".to_string());
    sm.push_pending_message(&sid, msg).await.unwrap();

    let popped = sm.pop_pending_message(&sid).await;
    assert!(popped.is_some());

    let again = sm.pop_pending_message(&sid).await;
    assert!(again.is_none(), "queue should be empty after single pop");
}
