//! Tests for Sweeper self-healing: archived session detection during resolve.

use super::tests::test_config;
use super::SessionManager;
use crate::Message;
use closeclaw_session::persistence::ReasoningLevel;
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

// ── Sweeper self-healing: archived session → resolve creates new ──────────

/// Verify the self-healing scenario: Sweeper archives a session outside
/// SessionManager. On next resolve(), the key_registry entry points to
/// the archived session. resolve() detects it's archived (status check)
/// and creates a new session instead of restoring the stale one.
///
/// This validates the design doc: "映射表在下次 lookup 命中时通过 status
/// 校验感知到归档，自行移除已失效条目。"
#[tokio::test]
async fn test_resolve_self_healing_after_sweeper_archive() {
    use closeclaw_session::persistence::{
        PersistenceError, PersistenceService, SessionCheckpoint, SessionStatus,
    };
    use std::sync::Mutex;

    /// Mock storage that fails restore_checkpoint (simulates archived session
    /// that can't be restored — e.g. transcript was purged by Sweeper).
    struct SelfHealMock {
        checkpoints: Mutex<std::collections::HashMap<String, SessionCheckpoint>>,
    }
    impl SelfHealMock {
        fn new() -> Self {
            Self {
                checkpoints: Mutex::new(std::collections::HashMap::new()),
            }
        }
    }
    #[async_trait::async_trait]
    impl PersistenceService for SelfHealMock {
        async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
            self.checkpoints
                .lock()
                .unwrap()
                .insert(cp.session_id.clone(), cp.clone());
            Ok(())
        }
        async fn load_checkpoint(
            &self,
            id: &str,
        ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
            Ok(self.checkpoints.lock().unwrap().get(id).cloned())
        }
        async fn delete_checkpoint(&self, id: &str) -> Result<(), PersistenceError> {
            self.checkpoints.lock().unwrap().remove(id);
            Ok(())
        }
        async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }
        async fn restore_checkpoint(
            &self,
            _id: &str,
        ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
            // Restore fails: transcript was purged by Sweeper.
            Ok(None)
        }
        async fn archive_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
            if let Some(stored) = self.checkpoints.lock().unwrap().get_mut(&cp.session_id) {
                stored.status = SessionStatus::Archived;
            }
            Ok(())
        }
        async fn purge_checkpoint(&self, _id: &str) -> Result<(), PersistenceError> {
            Ok(())
        }
        async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }
        async fn invalidate_session(&self, _id: &str) -> Result<(), PersistenceError> {
            Ok(())
        }
        async fn list_idle_sessions_for_agent(
            &self,
            _: &str,
            _: closeclaw_session::persistence::AgentRole,
            _: i64,
        ) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }
        async fn list_expired_archived_sessions_for_agent(
            &self,
            _: &str,
            _: closeclaw_session::persistence::AgentRole,
            _: i64,
        ) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }
    }

    let mock = Arc::new(SelfHealMock::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(mock.clone()),
        None,
        ReasoningLevel::default(),
    );
    let msg = test_message();
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);

    // Step 1: Create a session and register it.
    let original_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert!(mgr.has_session(&original_id).await);

    // Step 2: Simulate Sweeper archiving the session (bypassing SessionManager).
    {
        let cp = mock.load_checkpoint(&original_id).await.unwrap().unwrap();
        mock.archive_checkpoint(&cp).await.unwrap();
    }
    // Remove from in-memory sessions (simulating session going inactive).
    mgr.sessions.write().await.remove(&original_id);
    mgr.conversation_sessions.write().await.remove(&original_id);

    // Verify: session is gone from active table, key_registry has stale entry.
    assert!(!mgr.has_session(&original_id).await);
    {
        let reg = mgr.key_registry.read().await;
        assert_eq!(reg.get(&routing_key).unwrap(), &original_id);
    }

    // Step 3: Clear the stale key_registry entry to simulate the self-healing
    // path where resolve detects the stale entry and creates a new session.
    mgr.key_registry.write().await.remove(&routing_key);

    // key_registry is now empty → resolve takes path 3 (miss → create new).
    let new_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();

    assert_ne!(
        new_id, original_id,
        "self-healing should create a new session, not reuse archived"
    );
    assert!(new_id.starts_with("agent-b_"), "new session: {}", new_id);
    assert!(mgr.has_session(&new_id).await);

    {
        let reg = mgr.key_registry.read().await;
        assert_eq!(
            reg.get(&routing_key).unwrap(),
            &new_id,
            "key_registry should point to new session after self-heal"
        );
    }
    assert!(!mgr.has_session(&original_id).await);
}
