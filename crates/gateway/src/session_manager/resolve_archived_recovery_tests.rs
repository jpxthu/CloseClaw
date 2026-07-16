//! Tests for archived session recovery in resolve.rs Path 3.

use super::tests::test_config;
use super::SessionManager;
use crate::Message;
use closeclaw_session::persistence::{
    PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint, SessionStatus,
};
use std::sync::Arc;

fn test_message() -> Message {
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

/// Mock persistence service that supports archived session routing queries
/// and checkpoint loading for the resolve archived path tests.
struct ArchivedRoutingMock {
    /// Session IDs returned by `find_archived_session_by_routing`.
    archived_ids: std::sync::Mutex<Vec<String>>,
    /// Checkpoint to return from `load_checkpoint`.
    checkpoint: tokio::sync::Mutex<Option<SessionCheckpoint>>,
    /// Whether `restore_checkpoint` was called.
    restore_called: std::sync::Mutex<bool>,
}

#[async_trait::async_trait]
impl PersistenceService for ArchivedRoutingMock {
    async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        // Return cloned checkpoint on every call (not consuming it) so that
        // both try_restore_archived_session_inner and cm.load() succeed.
        Ok(self.checkpoint.lock().await.clone())
    }
    async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn restore_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        *self.restore_called.lock().unwrap() = true;
        Ok(self.checkpoint.lock().await.clone())
    }
    async fn find_archived_session_by_routing(
        &self,
        _account_id: Option<&str>,
        _channel: &str,
        _sender_id: &str,
        _peer_id: &str,
    ) -> Result<Option<String>, PersistenceError> {
        let mut ids = self.archived_ids.lock().unwrap();
        Ok(ids.pop())
    }
}

/// key_registry miss + active miss + archived hit → restore archived session.
#[tokio::test]
async fn test_resolve_path3_archived_hit_restores() {
    let session_id = "archived-restored".to_string();
    let mut cp = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Archived)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp.sender_id = Some("user-a".to_string());
    cp.account_id = None;

    let mock = Arc::new(ArchivedRoutingMock {
        archived_ids: std::sync::Mutex::new(vec![session_id.clone()]),
        checkpoint: tokio::sync::Mutex::new(Some(cp)),
        restore_called: std::sync::Mutex::new(false),
    });
    let mgr = SessionManager::new(
        &test_config(),
        Some(mock.clone()),
        None,
        ReasoningLevel::default(),
    );

    let msg = test_message();
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);

    // key_registry is empty → miss
    {
        let reg = mgr.key_registry.read().await;
        assert!(!reg.contains_key(&routing_key));
    }

    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(resolved, session_id);

    // restore_checkpoint was called
    assert!(*mock.restore_called.lock().unwrap());

    // Session registered in key_registry
    let reg = mgr.key_registry.read().await;
    assert_eq!(reg.get(&routing_key).unwrap(), &session_id);
}

/// key_registry miss + active miss + archived miss → create new session.
#[tokio::test]
async fn test_resolve_path3_archived_miss_creates_new() {
    let mock = Arc::new(ArchivedRoutingMock {
        archived_ids: std::sync::Mutex::new(vec![]),
        checkpoint: tokio::sync::Mutex::new(None),
        restore_called: std::sync::Mutex::new(false),
    });
    let mgr = SessionManager::new(
        &test_config(),
        Some(mock.clone()),
        None,
        ReasoningLevel::default(),
    );

    let msg = test_message();
    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();

    // Should create a new session (not archived)
    assert!(result.starts_with("agent-b_"), "new session: {}", result);
    assert!(!*mock.restore_called.lock().unwrap());
    assert!(mgr.has_session(&result).await);
}

/// Multiple archived session matches → takes last_message_at most recent.
#[tokio::test]
async fn test_resolve_path3_archived_multiple_matches_takes_newest() {
    let session_id_new = "archived-newest".to_string();
    let mut cp = SessionCheckpoint::new(session_id_new.clone())
        .with_status(SessionStatus::Archived)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp.sender_id = Some("user-a".to_string());
    cp.account_id = None;

    let mock = Arc::new(ArchivedRoutingMock {
        archived_ids: std::sync::Mutex::new(vec![
            "archived-old".to_string(),
            session_id_new.clone(),
        ]),
        checkpoint: tokio::sync::Mutex::new(Some(cp)),
        restore_called: std::sync::Mutex::new(false),
    });
    let mgr = SessionManager::new(
        &test_config(),
        Some(mock.clone()),
        None,
        ReasoningLevel::default(),
    );

    let msg = test_message();
    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    // The mock pops the last element first (archived-newest), so that's what gets restored.
    assert_eq!(
        resolved, session_id_new,
        "should restore the most recent archived session"
    );
}
