use super::*;
use crate::gateway::{GatewayConfig, Message};
use crate::session::persistence::{AgentRole, PersistenceError, SessionCheckpoint};
use async_trait::async_trait;
use tokio::sync::Mutex;

fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        dm_scope: DmScope::PerChannelPeer,
    }
}

fn test_message() -> Message {
    Message {
        id: "msg-1".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: std::collections::HashMap::new(),
    }
}

#[tokio::test]
async fn test_find_or_create_existing_session() {
    let mgr = SessionManager::new(&test_config(), None);
    let msg = test_message();
    let session_id = mgr.compute_session_key("feishu", &msg, None);
    {
        let mut sessions = mgr.sessions.write().await;
        sessions.insert(
            session_id.clone(),
            Session {
                id: session_id.clone(),
                agent_id: "agent-b".to_string(),
                channel: "feishu".to_string(),
                created_at: chrono::Utc::now().timestamp(),
            },
        );
    }

    // find_or_create should return the existing session (read-lock fast path)
    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(result, session_id);
}

#[tokio::test]
async fn test_find_or_create_new_user_creates_session() {
    let mgr = SessionManager::new(&test_config(), None);
    let msg = test_message();

    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let sessions = mgr.sessions.read().await;
    assert!(sessions.contains_key(&result));
    let session = sessions.get(&result).unwrap();
    assert_eq!(session.agent_id, "agent-b");
    assert_eq!(session.channel, "feishu");
}

#[tokio::test]
async fn test_archived_session_restoration() {
    use crate::session::persistence::SessionCheckpoint;
    use std::sync::Arc;

    // Build a mock storage that returns an Archived checkpoint
    let mock_storage = Arc::new(MockPersistService {
        archived_checkpoint: Mutex::new(Some(
            SessionCheckpoint::new("feishu:user-a:agent-b".to_string())
                .with_status(SessionStatus::Archived)
                .with_chat_id("agent-b".to_string()),
        )),
        restore_called: Mutex::new(false),
    });

    let config = test_config();
    let mut mgr = SessionManager::new(&config, Some(mock_storage.clone()));
    let msg = test_message();

    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(result, "feishu:user-a:agent-b");

    // Verify restore was called
    let called = *mock_storage.restore_called.lock().await;
    assert!(called, "restore_checkpoint should have been called");
}

#[tokio::test]
async fn test_find_or_create_no_storage() {
    // When storage is None, archived restoration returns false,
    // and find_or_create should still successfully create a new session.
    let mgr = SessionManager::new(&test_config(), None);
    let msg = test_message();

    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(result, "feishu:user-a:agent-b");

    let sessions = mgr.sessions.read().await;
    assert!(sessions.contains_key(&result));
    let session = sessions.get(&result).unwrap();
    assert_eq!(session.agent_id, "agent-b");
    assert_eq!(session.channel, "feishu");
}

// Mock persistence service for tests
struct MockPersistService {
    archived_checkpoint: Mutex<Option<crate::session::persistence::SessionCheckpoint>>,
    restore_called: Mutex<bool>,
}

#[async_trait::async_trait]
impl PersistenceService for MockPersistService {
    async fn save_checkpoint(
        &self,
        _checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.archived_checkpoint.lock().await.take())
    }

    async fn delete_checkpoint(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn restore_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        *self.restore_called.lock().await = true;
        Ok(self.archived_checkpoint.lock().await.take())
    }

    async fn archive_checkpoint(
        &self,
        _checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn purge_checkpoint(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn invalidate_session(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
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

// ── flush_all tests ───────────────────────────────────────────────────

/// Mock storage that tracks save_checkpoint calls and can simulate failures.
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
    fn with_failing_sessions(session_ids: &[&str]) -> Self {
        let fail_session_ids = session_ids
            .iter()
            .map(|s| (*s).to_string())
            .collect::<std::collections::HashSet<_>>();
        Self {
            saved_checkpoints: Mutex::new(Vec::new()),
            fail_session_ids,
        }
    }
}

#[async_trait::async_trait]
impl PersistenceService for FlushAllMockStorage {
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

    async fn restore_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }

    async fn archive_checkpoint(
        &self,
        _checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn purge_checkpoint(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn invalidate_session(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
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
    // storage is None → flush_all returns Ok(0)
    let mgr = SessionManager::new(&test_config(), None);
    let result = mgr.flush_all().await;
    assert!(
        result.is_ok(),
        "flush_all should succeed even with no storage"
    );
    assert_eq!(result.unwrap(), 0, "should return 0 when storage is None");
}

#[tokio::test]
async fn test_flush_all_empty_sessions() {
    // no active sessions → flush_all returns Ok(0)
    let storage = Arc::new(FlushAllMockStorage::new());
    let mgr = SessionManager::new(&test_config(), Some(storage.clone()));
    let result = mgr.flush_all().await;
    assert!(result.is_ok(), "flush_all should succeed with no sessions");
    assert_eq!(result.unwrap(), 0, "should return 0 when no sessions exist");
}

#[tokio::test]
async fn test_flush_all_saves_checkpoints() {
    // 3 sessions → all 3 checkpoints saved, returns 3
    let storage = Arc::new(FlushAllMockStorage::new());
    let mgr = SessionManager::new(&test_config(), Some(storage.clone()));

    let session_ids: Vec<String> = vec![
        "session-a".to_string(),
        "session-b".to_string(),
        "session-c".to_string(),
    ];
    {
        let mut sessions = mgr.sessions.write().await;
        for sid in &session_ids {
            sessions.insert(
                sid.clone(),
                Session {
                    id: sid.clone(),
                    agent_id: format!("agent-for-{}", sid),
                    channel: "feishu".to_string(),
                    created_at: chrono::Utc::now().timestamp(),
                },
            );
        }
    }

    let result = mgr.flush_all().await;
    assert!(result.is_ok(), "flush_all should succeed");
    assert_eq!(result.unwrap(), 3, "should return count of saved sessions");

    // Verify all checkpoints were saved with correct data
    let saved = storage.saved_checkpoints.lock().await;
    assert_eq!(saved.len(), 3, "all 3 checkpoints should be saved");

    let saved_ids: std::collections::HashSet<_> =
        saved.iter().map(|cp| cp.session_id.clone()).collect();
    for sid in &session_ids {
        assert!(saved_ids.contains(sid), "session {} should be saved", sid);
    }

    // Verify checkpoint fields are populated correctly
    for cp in saved.iter() {
        assert_eq!(cp.status, SessionStatus::Active);
        assert_eq!(cp.channel.as_ref(), Some(&"feishu".to_string()));
        assert!(cp.chat_id.is_some());
    }
}

#[tokio::test]
async fn test_flush_all_partial_failure() {
    // session-b fails to save → 2 saved, returns 2, no panic
    let storage = Arc::new(FlushAllMockStorage::with_failing_sessions(&["session-b"]));
    let mgr = SessionManager::new(&test_config(), Some(storage.clone()));

    let session_ids: Vec<String> = vec![
        "session-a".to_string(),
        "session-b".to_string(),
        "session-c".to_string(),
    ];
    {
        let mut sessions = mgr.sessions.write().await;
        for sid in &session_ids {
            sessions.insert(
                sid.clone(),
                Session {
                    id: sid.clone(),
                    agent_id: format!("agent-for-{}", sid),
                    channel: "feishu".to_string(),
                    created_at: chrono::Utc::now().timestamp(),
                },
            );
        }
    }

    let result = mgr.flush_all().await;
    assert!(
        result.is_ok(),
        "flush_all should succeed even when some saves fail"
    );
    assert_eq!(
        result.unwrap(),
        2,
        "should return count of successful saves"
    );

    // Verify only 2 checkpoints were saved (session-a and session-c)
    let saved = storage.saved_checkpoints.lock().await;
    assert_eq!(saved.len(), 2, "only successful saves should be recorded");

    let saved_ids: std::collections::HashSet<_> =
        saved.iter().map(|cp| cp.session_id.clone()).collect();
    assert!(saved_ids.contains("session-a"));
    assert!(saved_ids.contains("session-c"));
    assert!(
        !saved_ids.contains("session-b"),
        "session-b should NOT be saved"
    );
}

// ── get_chat_id tests ──────────────────────────────────────────────────

#[tokio::test]
async fn test_session_manager_get_chat_id() {
    let mgr = SessionManager::new(&test_config(), None);
    let msg = test_message();
    let sid = mgr.find_or_create("feishu", &msg, None).await.unwrap();

    // get_chat_id should return the agent_id field (= chat_id)
    let chat_id = mgr.get_chat_id(&sid).await;
    assert!(chat_id.is_some(), "expected Some(chat_id), got None");
    assert_eq!(chat_id.unwrap(), "agent-b");
}

#[tokio::test]
async fn test_session_manager_get_chat_id_missing() {
    let mgr = SessionManager::new(&test_config(), None);

    // Non-existent session_id → None
    let chat_id = mgr.get_chat_id("nonexistent-session-id").await;
    assert!(
        chat_id.is_none(),
        "expected None for missing session, got {chat_id:?}"
    );
}
