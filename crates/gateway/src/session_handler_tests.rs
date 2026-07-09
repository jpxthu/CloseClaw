use super::*;
use crate::session_handler::ActiveSearcherLlmCaller;
use closeclaw_common::LlmCaller;
use closeclaw_llm::fallback::FallbackClient;
use closeclaw_llm::retry::CooldownManager;
use closeclaw_llm::session_state::LlmState;
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;
use closeclaw_llm::LLMRegistry;
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::llm_session::ChatSession;
use closeclaw_session::persistence::ReasoningLevel;
use closeclaw_tasks::{
    BackgroundTask, BackgroundTaskError, CompletionNotification, NotificationPriority, TaskManager,
    TaskState,
};

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

// =========================================================================
// Task notification drain (Step 1.2 / Step 1.3)
// =========================================================================

/// A minimal [`TaskManager`] stub for gateway-level tests.
struct MockTaskManager {
    notifications: tokio::sync::Mutex<Vec<CompletionNotification>>,
}

impl MockTaskManager {
    fn with_notifications(notifs: Vec<CompletionNotification>) -> Self {
        Self {
            notifications: tokio::sync::Mutex::new(notifs),
        }
    }

    fn empty() -> Self {
        Self::with_notifications(vec![])
    }
}

#[async_trait::async_trait]
impl TaskManager for MockTaskManager {
    async fn spawn_task(
        &self,
        _command: &str,
        _cwd: &std::path::Path,
    ) -> Result<BackgroundTask, BackgroundTaskError> {
        unimplemented!("not needed for gateway tests")
    }
    async fn backgroundize_task(
        &self,
        _child: tokio::process::Child,
        _command: &str,
    ) -> Result<BackgroundTask, BackgroundTaskError> {
        unimplemented!("not needed for gateway tests")
    }
    async fn kill_task(&self, _task_id: &str) -> Result<(), BackgroundTaskError> {
        unimplemented!("not needed for gateway tests")
    }
    async fn get_task(&self, _task_id: &str) -> Option<BackgroundTask> {
        unimplemented!("not needed for gateway tests")
    }
    async fn drain_notifications(&self) -> Vec<CompletionNotification> {
        std::mem::take(&mut *self.notifications.lock().await)
    }
    async fn cleanup_finished(&self) {
        // no-op for gateway tests
    }
}

/// Build a [`CompletionNotification`] for testing.
fn make_notification(
    task_id: &str,
    command: &str,
    state: TaskState,
    output_path: std::path::PathBuf,
) -> CompletionNotification {
    CompletionNotification {
        task_id: task_id.to_owned(),
        command: command.to_owned(),
        state,
        output_path,
        priority: NotificationPriority::Later,
    }
}

/// Set up a session with a [`ConversationSession`] on the given
/// [`SessionManager`]. Returns the session_id.
async fn setup_session_with_conv(sm: &SessionManager, sid: &str) -> String {
    use crate::Session;
    use chrono::Utc;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    sm.sessions.write().await.insert(
        sid.to_string(),
        Session {
            id: sid.to_string(),
            agent_id: "test-agent".to_string(),
            channel: "test".to_string(),
            created_at: Utc::now().timestamp(),
            depth: 0,
        },
    );
    let cs = Arc::new(RwLock::new(
        closeclaw_session::llm_session::ConversationSession::new(
            sid.to_string(),
            "test-model".to_string(),
            std::path::PathBuf::from("/tmp"),
        ),
    ));
    sm.conversation_sessions
        .write()
        .await
        .insert(sid.to_string(), cs);
    sid.to_string()
}

/// When a task_manager with pending notifications is set on
/// SessionManager, `drain_announce_events` must drain them and inject
/// each as a `role="system"` message in the ConversationSession.
#[tokio::test]
async fn test_drain_notifications_injects_system_message() {
    let sm = make_sm();
    let sid = setup_session_with_conv(&sm, "notif-test").await;

    let tmp = tempfile::TempDir::new().unwrap();
    let output = tmp.path().join("closeclaw/background/t-1/output");
    let notif = make_notification(
        "t-1",
        "echo hello",
        TaskState::Completed { exit_code: 0 },
        output,
    );
    let tm: Arc<dyn TaskManager> = Arc::new(MockTaskManager::with_notifications(vec![notif]));
    sm.set_task_manager(tm).await;

    // handler_with_sm sets up LLM caller on SessionManager — needed
    // so the ConversationSession can invoke LLM when needed.
    let _handler = handler_with_sm(sm.clone()).await;
    SessionMessageHandler::drain_announce_events(&sm, &sid).await;

    let cs = sm.get_conversation_session(&sid).await.expect("session");
    let msgs = cs.read().await.messages().to_vec();
    assert!(!msgs.is_empty(), "should have at least one system message");

    let system_msgs: Vec<_> = msgs.iter().filter(|m| m.role == "system").collect();
    assert_eq!(
        system_msgs.len(),
        1,
        "expected exactly one system message, got {}",
        system_msgs.len()
    );

    let text = match &system_msgs[0].content_blocks[0] {
        closeclaw_llm::types::ContentBlock::Text(t) => t.clone(),
        other => panic!("expected Text block, got {:?}", other),
    };
    assert!(
        text.contains("t-1"),
        "should contain task_id, got: {}",
        text
    );
    assert!(
        text.contains("echo hello"),
        "should contain command, got: {}",
        text
    );
    assert!(
        text.contains("Completed"),
        "should contain state, got: {}",
        text
    );
}

/// When no task_manager is set on SessionManager, `drain_announce_events`
/// must return without panic or error.
#[tokio::test]
async fn test_drain_notifications_no_task_manager() {
    let sm = make_sm();
    let sid = setup_session_with_conv(&sm, "no-tm-test").await;
    // Do NOT set task_manager — it should be None by default.

    let _handler = handler_with_sm(sm.clone()).await;
    SessionMessageHandler::drain_announce_events(&sm, &sid).await;

    // No panic, no error. Session should still exist.
    assert!(sm.get_conversation_session(&sid).await.is_some());
}

/// When the task_manager has no pending notifications, `drain_announce_events`
/// must not inject any system messages.
#[tokio::test]
async fn test_drain_notifications_empty() {
    let sm = make_sm();
    let sid = setup_session_with_conv(&sm, "empty-notif-test").await;

    let tm: Arc<dyn TaskManager> = Arc::new(MockTaskManager::empty());
    sm.set_task_manager(tm).await;

    let _handler = handler_with_sm(sm.clone()).await;
    SessionMessageHandler::drain_announce_events(&sm, &sid).await;

    let cs = sm.get_conversation_session(&sid).await.expect("session");
    let msgs = cs.read().await.messages().to_vec();
    assert!(
        msgs.is_empty(),
        "no system messages should be injected, got {}",
        msgs.len()
    );
}
