//! Tests for outbound message active write behavior (Step 1.4).
//!
//! Verifies that outbound messages pushed via `push_pending_message`
//! are registered in `pending_operations` in the checkpoint, enabling
//! crash recovery to detect in-flight outbound messages.
//!
//! Behaviour dimensions:
//! 1. Push registers OutboundMessage in pending_operations
//! 2. Push persists checkpoint synchronously before returning
//! 3. Multiple pushes accumulate in pending_operations
//! 4. push_pending_message works without checkpoint_manager (no-op)

use super::tests::{clear_global_prompt_state, make_test_mgr};
use super::SessionManager;
use closeclaw_session::persistence::PendingMessage;
use closeclaw_session::persistence::{
    PendingOperationType, PersistenceError, PersistenceService, SessionCheckpoint,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

// ── Mock persistence service ──────────────────────────────────────────────

struct MockPersistence {
    checkpoints: Mutex<HashMap<String, SessionCheckpoint>>,
}

impl MockPersistence {
    fn new() -> Self {
        Self {
            checkpoints: Mutex::new(HashMap::new()),
        }
    }

    async fn insert_checkpoint(&self, cp: SessionCheckpoint) {
        self.checkpoints
            .lock()
            .await
            .insert(cp.session_id.clone(), cp);
    }
}

#[async_trait::async_trait]
impl PersistenceService for MockPersistence {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        self.checkpoints
            .lock()
            .await
            .insert(cp.session_id.clone(), cp.clone());
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.checkpoints.lock().await.get(session_id).cloned())
    }

    async fn delete_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn archive_checkpoint(&self, _cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn purge_checkpoint(&self, _id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
}

// ── Test helpers ──────────────────────────────────────────────────────────

/// Create a SessionManager with a mock persistence service.
async fn setup_with_mock_persistence() -> (Arc<SessionManager>, Arc<MockPersistence>) {
    let mgr = Arc::new(make_test_mgr(None));
    let mock = Arc::new(MockPersistence::new());
    let storage: Arc<dyn PersistenceService> = mock.clone() as Arc<dyn PersistenceService>;
    let cm = Arc::new(closeclaw_session::checkpoint_manager::CheckpointManager::new(storage));
    mgr.set_checkpoint_manager(cm).await;
    (mgr, mock)
}

/// Register a ConversationSession in the SessionManager's in-memory map.
/// Configures `checkpoint_storage` so that `persist_pending_checkpoint`
/// actually persists to the mock storage.
async fn register_conversation_session(
    mgr: &SessionManager,
    session_id: &str,
    storage: Arc<dyn PersistenceService>,
) {
    use closeclaw_session::llm_session::ConversationSession;
    let mut cs = ConversationSession::new(
        session_id.to_string(),
        "test-model".to_string(),
        std::path::PathBuf::from("/tmp"),
    );
    cs.set_checkpoint_storage(storage);
    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs));
    mgr.conversation_sessions
        .write()
        .await
        .insert(session_id.to_string(), cs_arc);
}

// ── Test 1: Push registers OutboundMessage in pending_operations ──────────

/// When a pending message is pushed, the checkpoint should contain
/// an OutboundMessage entry in `pending_operations`.
#[tokio::test]
async fn test_push_registers_outbound_in_pending_operations() {
    clear_global_prompt_state();

    let (mgr, mock) = setup_with_mock_persistence().await;
    let session_id = "aw-push-reg";

    // Register both sessions map and ConversationSession.
    mgr.sessions.write().await.insert(
        session_id.to_string(),
        super::Session {
            id: session_id.to_string(),
            agent_id: "test-agent".to_string(),
            channel: "feishu".to_string(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 0,
        },
    );
    register_conversation_session(
        &mgr,
        session_id,
        mock.clone() as Arc<dyn PersistenceService>,
    )
    .await;

    // Pre-populate checkpoint (needed for load_checkpoint in persist).
    mock.insert_checkpoint(SessionCheckpoint::new(session_id.to_string()))
        .await;

    // Push a pending message.
    let msg =
        PendingMessage::with_target_channel("msg-aw1".into(), "hello".into(), "feishu".into());
    mgr.push_pending_message(session_id, msg)
        .await
        .expect("push should succeed");

    // Verify the checkpoint has an OutboundMessage entry.
    let cp = mock
        .load_checkpoint(session_id)
        .await
        .unwrap()
        .expect("checkpoint should exist");

    let outbound_ops: Vec<_> = cp
        .pending_operations
        .iter()
        .filter(|op| op.op_type == PendingOperationType::OutboundMessage)
        .collect();

    assert_eq!(
        outbound_ops.len(),
        1,
        "should have exactly 1 OutboundMessage pending operation"
    );
    assert_eq!(outbound_ops[0].op_id, "msg-aw1");
}

// ── Test 2: Push persists checkpoint synchronously ───────────────────────

/// After `push_pending_message` returns, the checkpoint should already
/// be persisted (no async delay).
#[tokio::test]
async fn test_push_persists_checkpoint_synchronously() {
    clear_global_prompt_state();

    let (mgr, mock) = setup_with_mock_persistence().await;
    let session_id = "aw-push-sync";

    mgr.sessions.write().await.insert(
        session_id.to_string(),
        super::Session {
            id: session_id.to_string(),
            agent_id: "test-agent".to_string(),
            channel: "feishu".to_string(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 0,
        },
    );
    register_conversation_session(
        &mgr,
        session_id,
        mock.clone() as Arc<dyn PersistenceService>,
    )
    .await;
    mock.insert_checkpoint(SessionCheckpoint::new(session_id.to_string()))
        .await;

    let msg = PendingMessage::new("msg-sync".into(), "sync test".into());
    mgr.push_pending_message(session_id, msg)
        .await
        .expect("push should succeed");

    // Checkpoint should be saved immediately — verify by loading.
    let cp = mock
        .load_checkpoint(session_id)
        .await
        .unwrap()
        .expect("checkpoint should be persisted after push");

    assert!(
        !cp.pending_operations.is_empty(),
        "pending_operations should be non-empty after push"
    );
}

// ── Test 3: Multiple pushes accumulate ───────────────────────────────────

/// Multiple `push_pending_message` calls should accumulate
/// OutboundMessage entries in `pending_operations`.
#[tokio::test]
async fn test_multiple_pushes_accumulate_pending_operations() {
    clear_global_prompt_state();

    let (mgr, mock) = setup_with_mock_persistence().await;
    let session_id = "aw-push-multi";

    mgr.sessions.write().await.insert(
        session_id.to_string(),
        super::Session {
            id: session_id.to_string(),
            agent_id: "test-agent".to_string(),
            channel: "feishu".to_string(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 0,
        },
    );
    register_conversation_session(
        &mgr,
        session_id,
        mock.clone() as Arc<dyn PersistenceService>,
    )
    .await;
    mock.insert_checkpoint(SessionCheckpoint::new(session_id.to_string()))
        .await;

    // Push 3 messages.
    for i in 0..3 {
        let msg = PendingMessage::new(format!("msg-{}", i), format!("content {}", i));
        mgr.push_pending_message(session_id, msg)
            .await
            .expect("push should succeed");
    }

    let cp = mock
        .load_checkpoint(session_id)
        .await
        .unwrap()
        .expect("checkpoint should exist");

    let outbound_ops: Vec<_> = cp
        .pending_operations
        .iter()
        .filter(|op| op.op_type == PendingOperationType::OutboundMessage)
        .collect();

    assert_eq!(
        outbound_ops.len(),
        3,
        "should have 3 OutboundMessage pending operations"
    );

    let ids: Vec<&str> = outbound_ops.iter().map(|op| op.op_id.as_str()).collect();
    assert!(ids.contains(&"msg-0"));
    assert!(ids.contains(&"msg-1"));
    assert!(ids.contains(&"msg-2"));
}

// ── Test 4: No checkpoint_manager is no-op ───────────────────────────────

/// When no checkpoint_manager is set, `push_pending_message` should
/// still push to the unified queue but not fail.
#[tokio::test]
async fn test_push_no_checkpoint_manager_is_noop() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    // Intentionally do NOT set checkpoint_manager.

    let session_id = "aw-push-no-cp";
    mgr.sessions.write().await.insert(
        session_id.to_string(),
        super::Session {
            id: session_id.to_string(),
            agent_id: "test-agent".to_string(),
            channel: "feishu".to_string(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 0,
        },
    );
    register_conversation_session(
        &mgr,
        session_id,
        Arc::new(MockPersistence::new()) as Arc<dyn PersistenceService>,
    )
    .await;

    // Should not fail — push is still a no-op on persistence.
    let msg = PendingMessage::new("msg-nocp".into(), "no checkpoint".into());
    let result = mgr.push_pending_message(session_id, msg).await;
    assert!(
        result.is_ok(),
        "push should succeed without checkpoint_manager"
    );
}

// ── Test 5: Non-existent session returns error ───────────────────────────

/// When the session does not exist, `push_pending_message` should
/// return an error.
#[tokio::test]
async fn test_push_nonexistent_session_returns_error() {
    clear_global_prompt_state();

    let (mgr, _mock) = setup_with_mock_persistence().await;

    let msg = PendingMessage::new("msg-missing".into(), "missing".into());
    let result = mgr.push_pending_message("nonexistent-session", msg).await;
    assert!(result.is_err(), "should error for non-existent session");
    assert!(result.unwrap_err().contains("session not found"));
}

// ── Test 6: Crash recovery — pending operation survives checkpoint reload ─

/// Simulates a crash-and-recovery scenario: after pushing a pending
/// message, a new session loading the checkpoint from storage should
/// see the OutboundMessage entry in `pending_operations`.
#[tokio::test]
async fn test_crash_recovery_pending_operation_visible_after_reload() {
    clear_global_prompt_state();

    let (mgr, mock) = setup_with_mock_persistence().await;
    let session_id = "aw-crash-recovery";

    mgr.sessions.write().await.insert(
        session_id.to_string(),
        super::Session {
            id: session_id.to_string(),
            agent_id: "test-agent".to_string(),
            channel: "feishu".to_string(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 0,
        },
    );
    register_conversation_session(
        &mgr,
        session_id,
        mock.clone() as Arc<dyn PersistenceService>,
    )
    .await;
    mock.insert_checkpoint(SessionCheckpoint::new(session_id.to_string()))
        .await;

    // Push a pending message — this triggers checkpoint persist.
    let msg = PendingMessage::with_target_channel(
        "msg-crash".into(),
        "crash test".into(),
        "feishu".into(),
    );
    mgr.push_pending_message(session_id, msg)
        .await
        .expect("push should succeed");

    // Simulate crash recovery: load checkpoint from storage
    // and verify the pending operation is present.
    let restored_cp = mock
        .load_checkpoint(session_id)
        .await
        .unwrap()
        .expect("checkpoint should exist after crash recovery");

    let outbound_ops: Vec<_> = restored_cp
        .pending_operations
        .iter()
        .filter(|op| op.op_type == PendingOperationType::OutboundMessage)
        .collect();

    assert_eq!(
        outbound_ops.len(),
        1,
        "crash recovery should find 1 OutboundMessage pending operation"
    );
    assert_eq!(outbound_ops[0].op_id, "msg-crash");
}

// ── Test 7: Push fails when checkpoint persistence fails ─────────────────

/// A mock persistence service that always fails on save.
struct FailingPersistence;

#[async_trait::async_trait]
impl PersistenceService for FailingPersistence {
    async fn save_checkpoint(&self, _cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            "simulated save failure",
        )))
    }

    async fn load_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }

    async fn delete_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn archive_checkpoint(&self, _cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn purge_checkpoint(&self, _id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
}

/// When `persist_pending_checkpoint` fails, `push_pending_message`
/// should return an error.
#[tokio::test]
async fn test_push_fails_on_persistence_error() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let failing: Arc<dyn PersistenceService> = Arc::new(FailingPersistence);
    let cm =
        Arc::new(closeclaw_session::checkpoint_manager::CheckpointManager::new(failing.clone()));
    mgr.set_checkpoint_manager(cm).await;

    let session_id = "aw-push-fail-persist";
    mgr.sessions.write().await.insert(
        session_id.to_string(),
        super::Session {
            id: session_id.to_string(),
            agent_id: "test-agent".to_string(),
            channel: "feishu".to_string(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 0,
        },
    );
    register_conversation_session(&mgr, session_id, failing).await;

    let msg = PendingMessage::new("msg-fail".into(), "fail test".into());
    let result = mgr.push_pending_message(session_id, msg).await;
    assert!(
        result.is_err(),
        "push should fail when checkpoint persistence fails"
    );
    assert!(
        result.unwrap_err().contains("checkpoint persist failed"),
        "error message should mention checkpoint persist failure"
    );
}
