//! Tests for startup recovery notification injection.

use super::SessionManager;
use closeclaw_session::llm_session::{ChatSession, ConversationSession};
use closeclaw_session::persistence::{
    PersistenceError, PersistenceService, SessionCheckpoint, SessionStatus,
};
use std::sync::Arc;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Mock persistence
// ---------------------------------------------------------------------------

/// Mock persistence that stores checkpoints in memory and records saves.
struct RecoveryMockPersist {
    checkpoints: Mutex<std::collections::HashMap<String, SessionCheckpoint>>,
    saves: Arc<Mutex<Vec<SessionCheckpoint>>>,
}

impl RecoveryMockPersist {
    fn with_checkpoint(cp: SessionCheckpoint) -> Self {
        let mut map = std::collections::HashMap::new();
        map.insert(cp.session_id.clone(), cp);
        Self {
            checkpoints: Mutex::new(map),
            saves: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl PersistenceService for RecoveryMockPersist {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        self.saves.lock().await.push(cp.clone());
        self.checkpoints
            .lock()
            .await
            .insert(cp.session_id.clone(), cp.clone());
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.checkpoints.lock().await.get(sid).cloned())
    }
    async fn delete_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn purge_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn invalidate_session(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn archive_checkpoint(&self, _cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn restore_checkpoint(
        &self,
        _sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn list_idle_sessions_for_agent(
        &self,
        _a: &str,
        _r: closeclaw_session::persistence::AgentRole,
        _m: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_expired_archived_sessions_for_agent(
        &self,
        _a: &str,
        _r: closeclaw_session::persistence::AgentRole,
        _m: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
}

/// Helper: create a SessionManager with a mock persistence service.
fn make_recovery_test_mgr(persist: Arc<RecoveryMockPersist>) -> SessionManager {
    use closeclaw_session::persistence::ReasoningLevel;

    SessionManager::new(
        &super::tests::test_config(),
        Some(persist),
        None,
        ReasoningLevel::default(),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Verify that `inject_startup_recovery_notifications` creates a
/// `ConversationSession` and injects the recovery notification from
/// the checkpoint.
#[tokio::test]
async fn test_inject_recovery_notification_basic() {
    let cp = SessionCheckpoint::new("sess-1".to_string())
        .with_status(SessionStatus::Active)
        .with_agent_id("agent-1".to_string())
        .with_recovery_notification(Some("网关已重启，请检查未完成操作。".to_string()));
    let persist = Arc::new(RecoveryMockPersist::with_checkpoint(cp));
    let mgr = make_recovery_test_mgr(Arc::clone(&persist));

    mgr.inject_startup_recovery_notifications(&["sess-1".to_string()])
        .await;

    // ConversationSession should be created.
    let conv = mgr
        .get_conversation_session("sess-1")
        .await
        .expect("ConversationSession should exist");
    let conv = conv.read().await;
    let msgs = conv.messages();

    // Should contain the recovery notification as a system message.
    let sys_msg = msgs
        .iter()
        .find(|m| m.role == "system")
        .expect("should have a system message");
    match &sys_msg.content_blocks[0] {
        closeclaw_common::ContentBlock::Text(t) => {
            assert!(
                t.contains("网关已重启"),
                "system message should contain recovery text, got: {}",
                t
            );
        }
        other => panic!("expected Text block, got {:?}", other),
    }
}

/// Verify that tool failure results are injected as tool-role messages.
#[tokio::test]
async fn test_inject_tool_failures() {
    let tool_failure = serde_json::json!({
        "error": "进程中断：网关重启",
        "tool": "exec",
        "op_id": "op-123"
    })
    .to_string();
    let cp = SessionCheckpoint::new("sess-2".to_string())
        .with_status(SessionStatus::Active)
        .with_agent_id("agent-2".to_string())
        .with_recovery_notification(Some("重启通知".to_string()))
        .with_pending_tool_failures(vec![tool_failure]);
    let persist = Arc::new(RecoveryMockPersist::with_checkpoint(cp));
    let mgr = make_recovery_test_mgr(Arc::clone(&persist));

    mgr.inject_startup_recovery_notifications(&["sess-2".to_string()])
        .await;

    let conv = mgr
        .get_conversation_session("sess-2")
        .await
        .expect("ConversationSession should exist");
    let conv = conv.read().await;
    let msgs = conv.messages();

    // Should have a tool-role message with the failure content.
    let tool_msg = msgs
        .iter()
        .find(|m| m.role == "tool")
        .expect("should have a tool message");
    match &tool_msg.content_blocks[0] {
        closeclaw_common::ContentBlock::ToolResult { content, .. } => {
            assert!(
                content.contains("进程中断"),
                "tool result should contain failure text, got: {}",
                content
            );
        }
        other => panic!("expected ToolResult block, got {:?}", other),
    }
}

/// Verify that recovery data is cleared from the checkpoint after
/// injection to prevent double-injection.
#[tokio::test]
async fn test_clears_checkpoint_after_injection() {
    let cp = SessionCheckpoint::new("sess-3".to_string())
        .with_status(SessionStatus::Active)
        .with_agent_id("agent-3".to_string())
        .with_recovery_notification(Some("通知".to_string()));
    let persist = Arc::new(RecoveryMockPersist::with_checkpoint(cp));
    let mgr = make_recovery_test_mgr(Arc::clone(&persist));

    mgr.inject_startup_recovery_notifications(&["sess-3".to_string()])
        .await;

    // The checkpoint should have been saved with cleared recovery data.
    let saves = persist.saves.lock().await;
    let last_save = saves.last().expect("should have saved checkpoint");
    assert!(
        last_save.recovery_notification.is_none(),
        "recovery_notification should be cleared after injection"
    );
    assert!(
        last_save.pending_tool_failures.is_empty(),
        "pending_tool_failures should be cleared after injection"
    );
}

/// Verify that if a `ConversationSession` already exists, injection
/// is skipped (no double-injection).
#[tokio::test]
async fn test_skips_existing_conversation_session() {
    let cp = SessionCheckpoint::new("sess-4".to_string())
        .with_status(SessionStatus::Active)
        .with_agent_id("agent-4".to_string())
        .with_recovery_notification(Some("通知".to_string()));
    let persist = Arc::new(RecoveryMockPersist::with_checkpoint(cp));
    let mgr = make_recovery_test_mgr(Arc::clone(&persist));

    // Pre-create a ConversationSession.
    let conv = ConversationSession::new(
        "sess-4".to_string(),
        "test-model".to_string(),
        std::path::PathBuf::from("/tmp"),
    );
    mgr.conversation_sessions.write().await.insert(
        "sess-4".to_string(),
        Arc::new(tokio::sync::RwLock::new(conv)),
    );

    mgr.inject_startup_recovery_notifications(&["sess-4".to_string()])
        .await;

    // The existing ConversationSession should not have been modified.
    let conv = mgr.get_conversation_session("sess-4").await.unwrap();
    let conv = conv.read().await;
    let msgs = conv.messages();
    assert!(
        msgs.is_empty(),
        "existing ConversationSession should not be modified, found {} messages",
        msgs.len()
    );

    // Checkpoint should NOT have been saved (injection was skipped).
    let saves = persist.saves.lock().await;
    assert!(
        saves.is_empty(),
        "no checkpoint saves expected when session already exists"
    );
}

/// Verify that sessions without recovery data are skipped.
#[tokio::test]
async fn test_skips_clean_checkpoint() {
    let cp = SessionCheckpoint::new("sess-5".to_string())
        .with_status(SessionStatus::Active)
        .with_agent_id("agent-5".to_string());
    // No recovery_notification, no pending_tool_failures.
    let persist = Arc::new(RecoveryMockPersist::with_checkpoint(cp));
    let mgr = make_recovery_test_mgr(Arc::clone(&persist));

    mgr.inject_startup_recovery_notifications(&["sess-5".to_string()])
        .await;

    // No ConversationSession should be created.
    assert!(
        mgr.get_conversation_session("sess-5").await.is_none(),
        "clean checkpoint should not create a ConversationSession"
    );
}

/// Verify multiple dirty sessions are all processed.
#[tokio::test]
async fn test_multiple_dirty_sessions() {
    let cp1 = SessionCheckpoint::new("sess-a".to_string())
        .with_status(SessionStatus::Active)
        .with_agent_id("agent-a".to_string())
        .with_recovery_notification(Some("通知 A".to_string()));
    let cp2 = SessionCheckpoint::new("sess-b".to_string())
        .with_status(SessionStatus::Active)
        .with_agent_id("agent-b".to_string())
        .with_recovery_notification(Some("通知 B".to_string()));
    let persist = Arc::new(RecoveryMockPersist::with_checkpoint(cp1));
    // Insert second checkpoint.
    persist
        .checkpoints
        .lock()
        .await
        .insert("sess-b".to_string(), cp2);
    let mgr = make_recovery_test_mgr(Arc::clone(&persist));

    mgr.inject_startup_recovery_notifications(&["sess-a".to_string(), "sess-b".to_string()])
        .await;

    // Both sessions should have ConversationSessions.
    let conv_a = mgr.get_conversation_session("sess-a").await;
    let conv_b = mgr.get_conversation_session("sess-b").await;
    assert!(conv_a.is_some(), "sess-a should have ConversationSession");
    assert!(conv_b.is_some(), "sess-b should have ConversationSession");

    // Verify each has its own recovery notification.
    let msgs_a = conv_a.unwrap().read().await.messages().to_vec();
    let msgs_b = conv_b.unwrap().read().await.messages().to_vec();
    let sys_a = msgs_a.iter().find(|m| m.role == "system").unwrap();
    let sys_b = msgs_b.iter().find(|m| m.role == "system").unwrap();
    match (&sys_a.content_blocks[0], &sys_b.content_blocks[0]) {
        (closeclaw_common::ContentBlock::Text(a), closeclaw_common::ContentBlock::Text(b)) => {
            assert!(a.contains("通知 A"), "sess-a should have its notification");
            assert!(b.contains("通知 B"), "sess-b should have its notification");
        }
        other => panic!("unexpected content blocks: {:?}", other),
    }
}

/// Verify that a session entry is created in the sessions map
/// for the dirty session.
#[tokio::test]
async fn test_creates_session_entry() {
    let cp = SessionCheckpoint::new("sess-6".to_string())
        .with_status(SessionStatus::Active)
        .with_agent_id("agent-6".to_string())
        .with_recovery_notification(Some("通知".to_string()));
    let persist = Arc::new(RecoveryMockPersist::with_checkpoint(cp));
    let mgr = make_recovery_test_mgr(Arc::clone(&persist));

    mgr.inject_startup_recovery_notifications(&["sess-6".to_string()])
        .await;

    assert!(
        mgr.has_session("sess-6").await,
        "session entry should be created for dirty session"
    );
}

/// Empty dirty sessions list is a no-op.
#[tokio::test]
async fn test_empty_dirty_list() {
    let persist = Arc::new(RecoveryMockPersist::with_checkpoint(
        SessionCheckpoint::new("unused".to_string()),
    ));
    let mgr = make_recovery_test_mgr(Arc::clone(&persist));

    mgr.inject_startup_recovery_notifications(&[]).await;

    // No sessions should be created.
    assert!(mgr.get_all_sessions().await.is_empty());
}
