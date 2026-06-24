//! Tests for key_registry and resolve logic.

use super::session_helpers;
use super::test_helpers::MockPersistService;
use super::tests::{make_test_mgr, test_config};
use super::SessionManager;
use crate::gateway::{Message, Session};
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::{ReasoningLevel, SessionCheckpoint, SessionStatus};
use std::sync::Arc;
use tokio::sync::Mutex;

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

// ── generate_session_id ─────────────────────────────────────────────────────

#[test]
fn test_generate_session_id_format() {
    let id = session_helpers::generate_session_id("agent-1");
    let parts: Vec<&str> = id.split('_').collect();
    assert_eq!(parts.len(), 3, "expected 3 parts: {}", id);
    assert_eq!(parts[0], "agent-1");
    // Second part should be a numeric timestamp
    assert!(
        parts[1].parse::<i64>().is_ok(),
        "timestamp not numeric: {}",
        parts[1]
    );
    // Third part should be 8 hex digits
    assert_eq!(parts[2].len(), 8, "hex part not 8 chars: {}", parts[2]);
    assert!(
        parts[2].chars().all(|c| c.is_ascii_hexdigit()),
        "hex part non-hex: {}",
        parts[2]
    );
}

#[test]
fn test_generate_session_id_uniqueness() {
    let ids: Vec<String> = (0..100)
        .map(|_| session_helpers::generate_session_id("agent-x"))
        .collect();
    let unique: std::collections::HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();
    assert_eq!(unique.len(), 100, "all IDs should be unique");
}

// ── resolve: three paths ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_resolve_path1_active_hit() {
    let mgr = make_test_mgr(None);
    let msg = test_message();
    let session_key = mgr.compute_session_key("feishu", &msg, None, msg.timestamp);
    // resolve() strips timestamps before registry lookup — insert routing_key.
    let routing_key = SessionManager::strip_timestamp_from_session_key(&session_key);
    let session_id = "active_session_1";
    {
        let mut reg = mgr.key_registry.write().await;
        reg.insert(routing_key.to_string(), session_id.to_string());
    }
    {
        let mut sessions = mgr.sessions.write().await;
        sessions.insert(
            session_id.to_string(),
            Session {
                id: session_id.to_string(),
                agent_id: "agent-b".to_string(),
                channel: "feishu".to_string(),
                created_at: chrono::Utc::now().timestamp(),
                depth: 0,
            },
        );
    }
    let result = mgr
        .resolve(&session_key, "feishu", &msg, None)
        .await
        .unwrap();
    assert_eq!(result, session_id);
}

#[tokio::test]
async fn test_resolve_path2_archived_hit_restore() {
    let session_id = "archived_session_1";
    let mock_storage = Arc::new(MockPersistService {
        archived_checkpoint: Mutex::new(Some(
            SessionCheckpoint::new(session_id.to_string())
                .with_status(SessionStatus::Archived)
                .with_peer_id("agent-b".to_string()),
        )),
        restore_called: Mutex::new(false),
    });
    let mgr = SessionManager::new(
        &test_config(),
        Some(mock_storage.clone()),
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );
    let msg = test_message();
    let session_key = mgr.compute_session_key("feishu", &msg, None, msg.timestamp);
    // resolve() strips timestamps before registry lookup — insert routing_key.
    let routing_key = SessionManager::strip_timestamp_from_session_key(&session_key);
    {
        let mut reg = mgr.key_registry.write().await;
        reg.insert(routing_key.to_string(), session_id.to_string());
    }
    let result = mgr
        .resolve(&session_key, "feishu", &msg, None)
        .await
        .unwrap();
    assert_eq!(result, session_id);
    let called = *mock_storage.restore_called.lock().await;
    assert!(called, "restore_checkpoint should have been called");
}

#[tokio::test]
async fn test_resolve_path3_miss_creates_new() {
    let mgr = make_test_mgr(None);
    let msg = test_message();
    let session_key = mgr.compute_session_key("feishu", &msg, None, msg.timestamp);
    // key_registry is empty → miss → create new
    let result = mgr
        .resolve(&session_key, "feishu", &msg, None)
        .await
        .unwrap();
    // Verify format
    assert!(result.starts_with("agent-b_"), "bad format: {}", result);
    // Verify key_registry updated — resolve stores routing_key (timestamps stripped)
    let routing_key = SessionManager::strip_timestamp_from_session_key(&session_key);
    let reg = mgr.key_registry.read().await;
    assert_eq!(reg.get(routing_key).unwrap(), &result);
    // Verify session exists
    assert!(mgr.has_session(&result).await);
}

// ── rebuild_key_registry ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_rebuild_key_registry() {
    use crate::session::persistence::SessionCheckpoint;

    let mut cp1 = SessionCheckpoint::new("sid_old".to_string())
        .with_platform("feishu".to_string())
        .with_peer_id("agent-a".to_string())
        .with_agent_id("agent-a".to_string());
    cp1.created_at = chrono::Utc::now() - chrono::Duration::hours(2);
    let mut cp2 = SessionCheckpoint::new("sid_new".to_string())
        .with_platform("feishu".to_string())
        .with_peer_id("agent-a".to_string())
        .with_agent_id("agent-a".to_string());
    cp2.created_at = chrono::Utc::now();

    // Need to load_checkpoint to return the right checkpoint
    let cps = vec![cp1.clone(), cp2.clone()];
    struct RebuildMockWithLoad {
        checkpoints: Vec<SessionCheckpoint>,
    }
    #[async_trait::async_trait]
    impl crate::session::persistence::PersistenceService for RebuildMockWithLoad {
        async fn save_checkpoint(
            &self,
            _: &SessionCheckpoint,
        ) -> Result<(), crate::session::persistence::PersistenceError> {
            Ok(())
        }
        async fn load_checkpoint(
            &self,
            id: &str,
        ) -> Result<Option<SessionCheckpoint>, crate::session::persistence::PersistenceError>
        {
            Ok(self
                .checkpoints
                .iter()
                .find(|cp| cp.session_id == id)
                .cloned())
        }
        async fn delete_checkpoint(
            &self,
            _: &str,
        ) -> Result<(), crate::session::persistence::PersistenceError> {
            Ok(())
        }
        async fn list_active_sessions(
            &self,
        ) -> Result<Vec<String>, crate::session::persistence::PersistenceError> {
            Ok(self
                .checkpoints
                .iter()
                .map(|cp| cp.session_id.clone())
                .collect())
        }
        async fn list_archived_sessions(
            &self,
        ) -> Result<Vec<String>, crate::session::persistence::PersistenceError> {
            Ok(vec![])
        }
        async fn restore_checkpoint(
            &self,
            _: &str,
        ) -> Result<Option<SessionCheckpoint>, crate::session::persistence::PersistenceError>
        {
            Ok(None)
        }
    }

    let mock = Arc::new(RebuildMockWithLoad { checkpoints: cps });
    let mgr = SessionManager::new(
        &test_config(),
        Some(mock),
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );

    mgr.rebuild_key_registry().await.unwrap();

    let reg = mgr.key_registry.read().await;
    // Both sessions share the same reconstructed routing fields:
    // "default:feishu:agent-a:agent-a" (PerAccountChannelPeer, no sender_id → uses agent_id)
    // The registry key is now the sha256 hash of the routing fields.
    // The newer one (sid_new) should win.
    use sha2::{Digest, Sha256};
    let routing_fields = "default:feishu:agent-a:agent-a";
    let hash = Sha256::digest(routing_fields.as_bytes());
    let key = format!("{:x}", hash);
    assert_eq!(reg.get(&key).unwrap(), "sid_new");
}

// ── find_or_create delegates to resolve ──────────────────────────────────────

#[tokio::test]
async fn test_find_or_create_delegates_to_resolve() {
    let mgr = make_test_mgr(None);
    let msg = test_message();
    // First call creates a new session
    let id1 = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    // Second call with same message resolves to same session
    let id2 = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(id1, id2);
    // Only one session in the map
    let sessions = mgr.sessions.read().await;
    assert_eq!(sessions.len(), 1);
}

// ── key_registry write and query ─────────────────────────────────────────────

#[tokio::test]
async fn test_key_registry_write_and_query() {
    let mgr = make_test_mgr(None);
    // Write
    {
        let mut reg = mgr.key_registry.write().await;
        reg.insert("key1".to_string(), "sid1".to_string());
        reg.insert("key2".to_string(), "sid2".to_string());
    }
    // Query
    let reg = mgr.key_registry.read().await;
    assert_eq!(reg.get("key1").unwrap(), "sid1");
    assert_eq!(reg.get("key2").unwrap(), "sid2");
    assert!(reg.get("key3").is_none());
}
