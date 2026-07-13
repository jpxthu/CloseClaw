//! Tests for resolve() checkpoint status validation (Step 1.2).

use super::tests::test_config;
use super::SessionManager;
use crate::{Message, Session};
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

// ── Step 1.2: resolve() checkpoint status check ─────────────────────────────

/// Verify that when key_registry has a stale entry for an archived session
/// that is also in the in-memory sessions map, resolve() detects the archived
/// status. Path 1 removes the stale registry entry, then Path 2 restores the
/// archived session and re-registers the routing_key.
#[tokio::test]
async fn test_resolve_path1_archived_checkpoint_removes_stale_entry() {
    use closeclaw_session::persistence::{PersistenceService, SessionCheckpoint, SessionStatus};
    use closeclaw_session::storage::memory::MemoryStorage;

    let storage: Arc<MemoryStorage> = Arc::new(MemoryStorage::new());
    let mgr = Arc::new(SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        ReasoningLevel::default(),
    ));

    let session_id = "stale_archived_session".to_string();
    let msg = test_message();
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);

    // Save an archived checkpoint to storage
    let mut cp = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Archived)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp.sender_id = Some("user-a".to_string());
    storage.save_checkpoint(&cp).await.unwrap();

    // Register stale key_registry entry
    {
        let mut reg = mgr.key_registry.write().await;
        reg.insert(routing_key.clone(), session_id.clone());
    }
    // Put session in in-memory map (simulates Sweeper didn't clean it yet)
    {
        let mut sessions = mgr.sessions.write().await;
        sessions.insert(
            session_id.clone(),
            Session {
                id: session_id.clone(),
                agent_id: "agent-b".to_string(),
                channel: "feishu".to_string(),
                created_at: chrono::Utc::now().timestamp(),
                depth: 0,
            },
        );
    }

    // resolve(): Path 1 detects archived status and removes stale entry.
    // Path 2 then restores the archived session and re-registers the routing_key.
    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(
        result, session_id,
        "archived session should be restored via Path 2"
    );
    // Verify routing_key was re-registered after restore
    {
        let reg = mgr.key_registry.read().await;
        assert_eq!(
            reg.get(&routing_key).unwrap(),
            &session_id,
            "routing_key should be re-registered after restore"
        );
    }
}

/// Verify that when key_registry has an active session that is also in memory,
/// resolve() returns the existing session (no change to behavior).
#[tokio::test]
async fn test_resolve_path1_active_checkpoint_returns_existing() {
    use closeclaw_session::persistence::{PersistenceService, SessionCheckpoint, SessionStatus};
    use closeclaw_session::storage::memory::MemoryStorage;

    let storage: Arc<MemoryStorage> = Arc::new(MemoryStorage::new());
    let mgr = Arc::new(SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        ReasoningLevel::default(),
    ));

    let session_id = "active_session_check".to_string();
    let msg = test_message();
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);

    // Save an active checkpoint to storage
    let mut cp = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Active)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp.sender_id = Some("user-a".to_string());
    storage.save_checkpoint(&cp).await.unwrap();

    // Register key_registry entry
    {
        let mut reg = mgr.key_registry.write().await;
        reg.insert(routing_key.clone(), session_id.clone());
    }
    // Put session in in-memory map
    {
        let mut sessions = mgr.sessions.write().await;
        sessions.insert(
            session_id.clone(),
            Session {
                id: session_id.clone(),
                agent_id: "agent-b".to_string(),
                channel: "feishu".to_string(),
                created_at: chrono::Utc::now().timestamp(),
                depth: 0,
            },
        );
    }

    // resolve() should return the existing session
    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(result, session_id, "should return existing active session");
}

/// Verify that when checkpoint loading fails (storage error), resolve()
/// doesn't crash and falls back to returning the existing in-memory session.
#[tokio::test]
async fn test_resolve_path1_checkpoint_read_failure_returns_existing() {
    use closeclaw_session::persistence::{PersistenceError, SessionCheckpoint};

    struct FailingStorage;
    #[async_trait::async_trait]
    impl closeclaw_session::persistence::PersistenceService for FailingStorage {
        async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
            Ok(())
        }
        async fn load_checkpoint(
            &self,
            _: &str,
        ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
            Err(PersistenceError::Lock("simulated read failure".to_string()))
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
            Ok(None)
        }
    }

    let mock = Arc::new(FailingStorage);
    let mgr = Arc::new(SessionManager::new(
        &test_config(),
        Some(mock),
        None,
        ReasoningLevel::default(),
    ));

    let session_id = "failing_storage_session".to_string();
    let msg = test_message();
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);

    // Register key_registry entry and session in memory
    {
        let mut reg = mgr.key_registry.write().await;
        reg.insert(routing_key.clone(), session_id.clone());
    }
    {
        let mut sessions = mgr.sessions.write().await;
        sessions.insert(
            session_id.clone(),
            Session {
                id: session_id.clone(),
                agent_id: "agent-b".to_string(),
                channel: "feishu".to_string(),
                created_at: chrono::Utc::now().timestamp(),
                depth: 0,
            },
        );
    }

    // resolve() should not crash — returns existing session despite read failure
    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(
        result, session_id,
        "should return existing session despite checkpoint read failure"
    );
}

/// Verify that when key_registry has a stale entry for an archived session
/// but the session is NOT in the in-memory map (normal Sweeper scenario),
/// resolve() falls through to Path 2 and restores the session.
#[tokio::test]
async fn test_resolve_archived_not_in_memory_restores_via_path2() {
    use closeclaw_session::persistence::{PersistenceService, SessionCheckpoint, SessionStatus};
    use closeclaw_session::storage::memory::MemoryStorage;

    let storage: Arc<MemoryStorage> = Arc::new(MemoryStorage::new());
    let mgr = Arc::new(SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        ReasoningLevel::default(),
    ));

    let session_id = "archived_not_in_memory".to_string();
    let msg = test_message();
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);

    // Save an archived checkpoint to storage
    let mut cp = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Archived)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp.sender_id = Some("user-a".to_string());
    storage.save_checkpoint(&cp).await.unwrap();

    // Register key_registry entry but do NOT put in sessions map
    {
        let mut reg = mgr.key_registry.write().await;
        reg.insert(routing_key.clone(), session_id.clone());
    }
    // Session is NOT in the in-memory map (Sweeper cleaned it up)

    // resolve() should restore via Path 2
    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(result, session_id, "should restore the archived session");
    // Verify key_registry was re-registered
    {
        let reg = mgr.key_registry.read().await;
        assert_eq!(reg.get(&routing_key).unwrap(), &session_id);
    }
}
