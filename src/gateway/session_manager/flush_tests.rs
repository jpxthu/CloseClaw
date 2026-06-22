use super::*;
use crate::daemon::shutdown::ShutdownMode;
use crate::gateway::{GatewayConfig, Message};
use crate::llm::session::ConversationSession;
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::ReasoningLevel;
use crate::session::persistence::{AgentRole, PendingMessage, PersistenceError, SessionCheckpoint};
use async_trait::async_trait;
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
impl crate::session::persistence::PersistenceService for FlushAllMockStorage {
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
    let mgr = SessionManager::new(
        &test_config(),
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );
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
        BootstrapMode::Full,
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
        BootstrapMode::Full,
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
        BootstrapMode::Full,
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
    let mgr = SessionManager::new(
        &test_config(),
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );
    let msg = test_message();
    let sid = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let chat_id = mgr.get_chat_id(&sid).await;
    assert!(chat_id.is_some());
    assert_eq!(chat_id.unwrap(), "agent-b");
}

#[tokio::test]
async fn test_session_manager_get_chat_id_missing() {
    let mgr = SessionManager::new(
        &test_config(),
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );
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
        BootstrapMode::Full,
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
        BootstrapMode::Full,
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
        BootstrapMode::Full,
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

// rebuild tests moved to rebuild_tests.rs (file kept under 500 lines)
// Bug #904 tests moved to bug904_tests.rs
