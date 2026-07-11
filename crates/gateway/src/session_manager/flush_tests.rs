use super::*;
use crate::{GatewayConfig, Message};
use async_trait::async_trait;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::ReasoningLevel;
use closeclaw_session::persistence::{
    AgentRole, PendingMessage, PersistenceError, SessionCheckpoint,
};
use tokio::sync::Mutex;

pub(super) fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        dm_scope: DmScope::PerChannelPeer,
        ..Default::default()
    }
}

pub(super) fn test_message() -> Message {
    Message {
        id: "msg-1".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: std::collections::HashMap::new(),
        thread_id: None,
    }
}

pub(super) fn make_test_session(id: &str) -> Session {
    Session {
        id: id.to_string(),
        agent_id: "agent-b".to_string(),
        channel: "feishu".to_string(),
        created_at: chrono::Utc::now().timestamp(),
        depth: 0,
    }
}

struct FlushAllMockStorage {
    saved_checkpoints: Mutex<Vec<SessionCheckpoint>>,
    fail_session_ids: std::collections::HashSet<String>,
}

impl FlushAllMockStorage {
    fn new() -> Self {
        Self {
            saved_checkpoints: Mutex::new(Vec::new()),
            fail_session_ids: std::collections::HashSet::new(),
        }
    }
    fn with_failing_sessions(ids: &[&str]) -> Self {
        Self {
            saved_checkpoints: Mutex::new(Vec::new()),
            fail_session_ids: ids.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[async_trait]
impl closeclaw_session::persistence::PersistenceService for FlushAllMockStorage {
    async fn save_checkpoint(
        &self,
        checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        if self.fail_session_ids.contains(&checkpoint.session_id) {
            return Err(PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "simulated failure",
            )));
        }
        self.saved_checkpoints.lock().await.push(checkpoint.clone());
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }

    async fn delete_checkpoint(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn archive_checkpoint(
        &self,
        _checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn restore_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }

    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn list_idle_sessions_for_agent(
        &self,
        _agent_id: &str,
        _role: AgentRole,
        _idle_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn list_expired_archived_sessions_for_agent(
        &self,
        _agent_id: &str,
        _role: AgentRole,
        _purge_after_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn test_flush_all_no_storage() {
    let mgr = SessionManager::new(&test_config(), None, None, ReasoningLevel::default());
    let result = mgr.flush_all(ShutdownMode::Graceful).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0);
}

#[tokio::test]
async fn test_flush_all_empty_sessions() {
    let storage = Arc::new(FlushAllMockStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage),
        None,
        ReasoningLevel::default(),
    );
    let result = mgr.flush_all(ShutdownMode::Graceful).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0);
}

#[tokio::test]
async fn test_flush_all_saves_checkpoints() {
    let storage = Arc::new(FlushAllMockStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        ReasoningLevel::default(),
    );

    let session_ids = vec!["session-a", "session-b", "session-c"];
    {
        let mut sessions = mgr.sessions.write().await;
        for sid in &session_ids {
            sessions.insert(sid.to_string(), make_test_session(sid));
        }
    }

    let result = mgr.flush_all(ShutdownMode::Graceful).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 3);

    let saved = storage.saved_checkpoints.lock().await;
    assert_eq!(saved.len(), 3);
    let saved_ids: std::collections::HashSet<_> =
        saved.iter().map(|cp| cp.session_id.clone()).collect();
    for sid in &session_ids {
        assert!(saved_ids.contains(*sid));
    }
    for cp in saved.iter() {
        assert_eq!(cp.status, SessionStatus::Active);
        assert_eq!(cp.platform.as_ref(), Some(&"feishu".to_string()));
        assert!(cp.peer_id.is_some());
    }
}

#[tokio::test]
async fn test_flush_all_partial_failure() {
    let storage = Arc::new(FlushAllMockStorage::with_failing_sessions(&["session-b"]));
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        ReasoningLevel::default(),
    );

    let session_ids = vec!["session-a", "session-b", "session-c"];
    {
        let mut sessions = mgr.sessions.write().await;
        for sid in &session_ids {
            sessions.insert(sid.to_string(), make_test_session(sid));
        }
    }

    let result = mgr.flush_all(ShutdownMode::Graceful).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 2);

    let saved = storage.saved_checkpoints.lock().await;
    assert_eq!(saved.len(), 2);
}

#[tokio::test]
async fn test_session_manager_get_chat_id() {
    let mgr = SessionManager::new(&test_config(), None, None, ReasoningLevel::default());
    let msg = test_message();
    let sid = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let chat_id = mgr.get_chat_id(&sid).await;
    assert!(chat_id.is_some());
    assert_eq!(chat_id.unwrap(), "agent-b");
}

#[tokio::test]
async fn test_session_manager_get_chat_id_missing() {
    let mgr = SessionManager::new(&test_config(), None, None, ReasoningLevel::default());
    let chat_id = mgr.get_chat_id("nonexistent-session-id").await;
    assert!(chat_id.is_none());
}
// ── pending_messages flush scenarios ───────────────────────────────────────────

#[tokio::test]
async fn test_flush_all_with_pending_messages() {
    let storage = Arc::new(FlushAllMockStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        ReasoningLevel::default(),
    );

    let session_id = "session-with-pending";
    {
        let mut sessions = mgr.sessions.write().await;
        sessions.insert(session_id.to_string(), make_test_session(session_id));
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let conv_session = Arc::new(RwLock::new(ConversationSession::new(
        session_id.to_string(),
        "gpt-4o".to_string(),
        tmp.path().to_path_buf(),
    )));
    {
        let mut cs = conv_session.write().await;
        cs.push_pending(PendingMessage::new("msg_1".into(), "hello".into()));
        cs.push_pending(PendingMessage::new("msg_2".into(), "world".into()));
    }
    {
        let mut conv_sessions = mgr.conversation_sessions.write().await;
        conv_sessions.insert(session_id.to_string(), conv_session);
    }

    let result = mgr.flush_all(ShutdownMode::Graceful).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 1);

    let saved = storage.saved_checkpoints.lock().await;
    assert_eq!(saved.len(), 1);
    let cp = &saved[0];
    assert_eq!(cp.session_id, session_id);
    assert_eq!(
        cp.pending_messages.len(),
        2,
        "checkpoint should contain 2 pending messages"
    );
    assert_eq!(cp.pending_messages[0].message_id, "msg_1");
    assert_eq!(cp.pending_messages[1].message_id, "msg_2");
    assert!(!cp.pending_messages[0].sent);
    assert!(!cp.pending_messages[1].sent);
}

#[tokio::test]
async fn test_flush_all_without_pending_messages() {
    let storage = Arc::new(FlushAllMockStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        ReasoningLevel::default(),
    );

    let session_id = "session-no-pending";
    {
        let mut sessions = mgr.sessions.write().await;
        sessions.insert(session_id.to_string(), make_test_session(session_id));
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let conv_session = Arc::new(RwLock::new(ConversationSession::new(
        session_id.to_string(),
        "gpt-4o".to_string(),
        tmp.path().to_path_buf(),
    )));
    {
        let mut conv_sessions = mgr.conversation_sessions.write().await;
        conv_sessions.insert(session_id.to_string(), conv_session);
    }

    let result = mgr.flush_all(ShutdownMode::Graceful).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 1);

    let saved = storage.saved_checkpoints.lock().await;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].session_id, session_id);
    assert!(
        saved[0].pending_messages.is_empty(),
        "checkpoint pending_messages should be empty when ConversationSession has no pending"
    );
}

#[tokio::test]
async fn test_flush_all_no_conversation_session() {
    let storage = Arc::new(FlushAllMockStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        ReasoningLevel::default(),
    );

    let session_id = "session-no-conv";
    {
        let mut sessions = mgr.sessions.write().await;
        sessions.insert(session_id.to_string(), make_test_session(session_id));
    }

    let result = mgr.flush_all(ShutdownMode::Graceful).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 1);

    let saved = storage.saved_checkpoints.lock().await;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].session_id, session_id);
    assert!(
        saved[0].pending_messages.is_empty(),
        "checkpoint pending_messages should be empty when no ConversationSession exists"
    );
}

#[tokio::test]
async fn test_with_pending_messages_bulk_set() {
    let pm1 = PendingMessage::new("msg_1".into(), "first".into());
    let pm2 = PendingMessage::new("msg_2".into(), "second".into());
    let pm3 = PendingMessage::new("msg_3".into(), "third".into());

    let cp = SessionCheckpoint::new("sess-check".into())
        .add_pending_message(pm1)
        .with_pending_messages(vec![pm2, pm3]);

    assert_eq!(cp.pending_messages.len(), 2);
    assert_eq!(cp.pending_messages[0].message_id, "msg_2");
    assert_eq!(cp.pending_messages[1].message_id, "msg_3");
}

// ── Phase 4 fallback persistence tests ─────────────────────────────────────

/// Verify that after Phase 2 stops sessions, `flush_all` (Phase 4)
/// still finds them in the tracking tables and persists them.
///
/// This validates the fix: `stop_single_session` no longer calls
/// `remove_session`, so Phase 4 fallback persistence actually
/// processes stopped sessions.
#[tokio::test]
async fn test_flush_all_after_stop_preserves_sessions() {
    let storage = Arc::new(FlushAllMockStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        ReasoningLevel::default(),
    );

    let session_ids = vec!["stopped-a", "stopped-b"];
    for sid in &session_ids {
        // Register session in tracking table
        mgr.sessions
            .write()
            .await
            .insert(sid.to_string(), make_test_session(sid));
        // Register a ConversationSession (required by stop_single_session)
        let cs = Arc::new(tokio::sync::RwLock::new(
            closeclaw_session::llm_session::ConversationSession::new(
                sid.to_string(),
                "gpt-4o".to_string(),
                std::path::PathBuf::from("/tmp"),
            ),
        ));
        mgr.conversation_sessions
            .write()
            .await
            .insert(sid.to_string(), cs);
    }

    // Phase 2: stop all sessions
    use closeclaw_common::shutdown::ShutdownMode;
    let stop_result = mgr.stop_all_sessions(ShutdownMode::Forceful, None).await;
    assert_eq!(
        stop_result.succeeded, 2,
        "both sessions should be stopped successfully"
    );

    // Sessions should still be in tracking tables after stop
    assert!(
        mgr.has_session("stopped-a").await,
        "session-a should still exist after Phase 2 stop"
    );
    assert!(
        mgr.has_session("stopped-b").await,
        "session-b should still exist after Phase 2 stop"
    );

    // Phase 4: flush_all should find and persist the stopped sessions
    let saved = mgr.flush_all(ShutdownMode::Graceful).await;
    assert!(saved.is_ok());
    assert_eq!(
        saved.unwrap(),
        2,
        "flush_all should persist both stopped sessions"
    );

    // Verify checkpoints were saved
    let persisted = storage.saved_checkpoints.lock().await;
    let persisted_ids: std::collections::HashSet<_> =
        persisted.iter().map(|cp| cp.session_id.clone()).collect();
    for sid in &session_ids {
        assert!(
            persisted_ids.contains(*sid),
            "checkpoint for {} should have been persisted",
            sid
        );
    }
}

/// Verify that `flush_all` cleans up tracking tables after persistence.
///
/// After `flush_all` completes, all sessions should be removed from
/// `sessions`, `conversation_sessions`, and `channel_active_sessions`.
#[tokio::test]
async fn test_flush_all_clears_tracking_after_persist() {
    let storage = Arc::new(FlushAllMockStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        ReasoningLevel::default(),
    );

    let session_ids = vec!["persist-a", "persist-b", "persist-c"];
    for sid in &session_ids {
        mgr.sessions
            .write()
            .await
            .insert(sid.to_string(), make_test_session(sid));
        let cs = Arc::new(tokio::sync::RwLock::new(
            closeclaw_session::llm_session::ConversationSession::new(
                sid.to_string(),
                "gpt-4o".to_string(),
                std::path::PathBuf::from("/tmp"),
            ),
        ));
        mgr.conversation_sessions
            .write()
            .await
            .insert(sid.to_string(), cs);
    }
    // Register a channel_active_sessions entry for one session
    mgr.channel_active_sessions
        .write()
        .await
        .insert("feishu".to_string(), "persist-a".to_string());

    // Verify sessions exist before flush
    assert!(mgr.has_session("persist-a").await);
    assert!(mgr.has_session("persist-b").await);
    assert!(mgr.has_session("persist-c").await);

    // Phase 4: flush_all persists and cleans up
    let saved = mgr
        .flush_all(closeclaw_common::shutdown::ShutdownMode::Graceful)
        .await;
    assert!(saved.is_ok());
    assert_eq!(saved.unwrap(), 3);

    // Verify sessions are removed from sessions tracking table
    assert!(!mgr.has_session("persist-a").await);
    assert!(!mgr.has_session("persist-b").await);
    assert!(!mgr.has_session("persist-c").await);

    // Verify sessions are removed from conversation_sessions
    {
        let conv = mgr.conversation_sessions.read().await;
        assert!(!conv.contains_key("persist-a"));
        assert!(!conv.contains_key("persist-b"));
        assert!(!conv.contains_key("persist-c"));
    }

    // Verify channel_active_sessions is cleaned up
    {
        let channel_map = mgr.channel_active_sessions.read().await;
        assert!(
            channel_map.get("feishu").is_none(),
            "channel_active_sessions should be cleared after flush_all"
        );
    }
}

// rebuild tests moved to rebuild_tests.rs (file kept under 500 lines)
// Bug #904 tests moved to bug904_tests.rs
