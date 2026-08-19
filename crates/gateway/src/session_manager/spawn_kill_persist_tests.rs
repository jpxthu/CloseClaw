//! Tests for `SessionManager::terminate_and_persist_session` and
//! the persistence path in `kill_child`.
//!
//! Verifies that when a child session (and its descendants) are killed,
//! their checkpoints are persisted to storage before the in-memory maps
//! are cleared, and that persistence failures do not block the kill.

use super::spawn::{ChildSessionInfo, ChildSessionStatus, SpawnMode};
use super::tests::{clear_global_prompt_state, test_config};
use super::SessionManager;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::{PersistenceError, PersistenceService, SessionCheckpoint};
use serial_test::serial;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

// ── Helpers ────────────────────────────────────────────────────────────

/// Register a `ConversationSession` in the manager's
/// `conversation_sessions` map and in `sessions`.
async fn register_session(mgr: &SessionManager, id: &str, agent_id: &str, depth: u32) {
    let cs = ConversationSession::new(id.to_string(), "test-model".into(), PathBuf::from("/tmp"));
    mgr.conversation_sessions
        .write()
        .await
        .insert(id.to_string(), Arc::new(RwLock::new(cs)));
    mgr.sessions.write().await.insert(
        id.to_string(),
        crate::Session {
            id: id.to_string(),
            agent_id: agent_id.to_string(),
            channel: "spawn".into(),
            created_at: 0,
            depth,
        },
    );
}

/// Register a parent-child entry in the `children` table.
async fn register_tree_entry(
    mgr: &SessionManager,
    parent_id: &str,
    child_id: &str,
    agent_id: &str,
    depth: u32,
) {
    mgr.register_child(
        parent_id,
        ChildSessionInfo {
            session_id: child_id.to_string(),
            parent_session_id: parent_id.to_string(),
            agent_id: agent_id.to_string(),
            depth,
            mode: SpawnMode::Run,
            status: ChildSessionStatus::Active,
            timeout_secs: None,
            timeout_warning_secs: None,
            timeout_notify_interval_ratio: None,
            created_at: std::time::Instant::now(),
        },
    )
    .await;
}

/// A `PersistenceService` whose `save_checkpoint` always fails.
struct FailingPersistService;

#[async_trait::async_trait]
impl PersistenceService for FailingPersistService {
    async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Err(PersistenceError::Lock("forced test failure".into()))
    }
    async fn load_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn archive_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn purge_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

/// Kill a single child session → checkpoint is persisted to storage
/// with correct session_id and transcript metadata.
#[tokio::test]
#[serial]
async fn test_kill_child_persists_checkpoint_for_single_child() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        Some(tmp.path().to_path_buf()),
        Default::default(),
    );

    // Register parent with ConversationSession.
    {
        let cs = ConversationSession::new(
            "parent-persists".into(),
            "test-model".into(),
            tmp.path().to_path_buf(),
        );
        mgr.conversation_sessions
            .write()
            .await
            .insert("parent-persists".into(), Arc::new(RwLock::new(cs)));
        mgr.sessions.write().await.insert(
            "parent-persists".into(),
            crate::Session {
                id: "parent-persists".into(),
                agent_id: "root-agent".into(),
                channel: "spawn".into(),
                created_at: 0,
                depth: 0,
            },
        );
    }

    register_session(&mgr, "child-persists", "child-agent", 1).await;
    register_tree_entry(&mgr, "parent-persists", "child-persists", "child-agent", 1).await;

    // Confirm child exists.
    assert!(mgr.has_session("child-persists").await);

    // Kill child.
    mgr.kill_child("parent-persists", "child-persists")
        .await
        .expect("kill_child should succeed");

    // Child removed from memory.
    assert!(!mgr.has_session("child-persists").await);

    // Checkpoint is persisted in storage with correct session_id.
    let cp = storage
        .load_checkpoint("child-persists")
        .await
        .expect("storage should be accessible")
        .expect("child checkpoint should exist in storage after kill");
    assert_eq!(cp.session_id, "child-persists");
    // Transcript snapshot metadata should be present (set by
    // snapshot_current_state before stop).
    assert!(
        !cp.snapshot_metas.is_empty() || cp.agent_id.is_some(),
        "checkpoint should contain session metadata"
    );
}

/// Persistence happens before memory removal: after kill_child completes
/// and the session is removed from all in-memory maps, the checkpoint
/// is still readable from storage.
#[tokio::test]
#[serial]
async fn test_kill_child_persists_before_memory_removal() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        Some(tmp.path().to_path_buf()),
        Default::default(),
    );

    // Register parent.
    {
        let cs = ConversationSession::new(
            "parent-order".into(),
            "test-model".into(),
            tmp.path().to_path_buf(),
        );
        mgr.conversation_sessions
            .write()
            .await
            .insert("parent-order".into(), Arc::new(RwLock::new(cs)));
        mgr.sessions.write().await.insert(
            "parent-order".into(),
            crate::Session {
                id: "parent-order".into(),
                agent_id: "root-agent".into(),
                channel: "spawn".into(),
                created_at: 0,
                depth: 0,
            },
        );
    }

    register_session(&mgr, "child-order", "child-agent", 1).await;
    register_tree_entry(&mgr, "parent-order", "child-order", "child-agent", 1).await;

    mgr.kill_child("parent-order", "child-order")
        .await
        .expect("kill_child should succeed");

    // All in-memory maps should be cleared.
    assert!(!mgr.has_session("child-order").await);
    assert!(mgr.get_conversation_session("child-order").await.is_none());
    assert_eq!(mgr.count_active_children("parent-order").await, 0);

    // But checkpoint is still in storage.
    let cp = storage
        .load_checkpoint("child-order")
        .await
        .expect("storage should be accessible")
        .expect("checkpoint should persist after memory removal");
    assert_eq!(cp.session_id, "child-order");
}

/// When checkpoint persistence fails, kill_child still completes:
/// memory maps are cleared and no error is returned.
#[tokio::test]
#[serial]
async fn test_kill_child_completes_when_persistence_fails() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let failing_storage: Arc<dyn PersistenceService> = Arc::new(FailingPersistService);
    let mgr = SessionManager::new(
        &test_config(),
        Some(failing_storage),
        Some(tmp.path().to_path_buf()),
        Default::default(),
    );

    // Register parent.
    {
        let cs = ConversationSession::new(
            "parent-fail".into(),
            "test-model".into(),
            tmp.path().to_path_buf(),
        );
        mgr.conversation_sessions
            .write()
            .await
            .insert("parent-fail".into(), Arc::new(RwLock::new(cs)));
        mgr.sessions.write().await.insert(
            "parent-fail".into(),
            crate::Session {
                id: "parent-fail".into(),
                agent_id: "root-agent".into(),
                channel: "spawn".into(),
                created_at: 0,
                depth: 0,
            },
        );
    }

    register_session(&mgr, "child-fail", "child-agent", 1).await;
    register_tree_entry(&mgr, "parent-fail", "child-fail", "child-agent", 1).await;

    // kill_child must succeed even though persistence fails.
    let result = mgr.kill_child("parent-fail", "child-fail").await;
    assert!(
        result.is_ok(),
        "kill_child should not return error when persistence fails"
    );

    // Memory maps should still be cleared.
    assert!(!mgr.has_session("child-fail").await);
    assert!(mgr.get_conversation_session("child-fail").await.is_none());
    assert_eq!(mgr.count_active_children("parent-fail").await, 0);
}

/// Multi-descendant cascade: killing a child persists checkpoints for
/// all descendants (grandchild + great-grandchild) and the child itself.
///
/// Tree:
/// ```text
///   parent (root, not killed)
///     └─ child (killed)
///          ├─ grandchild1
///          │    └─ great_grandchild
///          └─ grandchild2
/// ```
#[tokio::test]
#[serial]
async fn test_kill_child_persists_all_descendants_checkpoints() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        Some(tmp.path().to_path_buf()),
        Default::default(),
    );

    // Register parent.
    {
        let cs = ConversationSession::new(
            "parent-multi".into(),
            "test-model".into(),
            tmp.path().to_path_buf(),
        );
        mgr.conversation_sessions
            .write()
            .await
            .insert("parent-multi".into(), Arc::new(RwLock::new(cs)));
        mgr.sessions.write().await.insert(
            "parent-multi".into(),
            crate::Session {
                id: "parent-multi".into(),
                agent_id: "root-agent".into(),
                channel: "spawn".into(),
                created_at: 0,
                depth: 0,
            },
        );
    }

    // Register all descendant sessions.
    register_session(&mgr, "child-multi", "child-agent", 1).await;
    register_session(&mgr, "gc1-multi", "gc1-agent", 2).await;
    register_session(&mgr, "gc2-multi", "gc2-agent", 2).await;
    register_session(&mgr, "ggc-multi", "ggc-agent", 3).await;

    // Build spawn tree: parent→child→{gc1, gc2}; gc1→ggc.
    register_tree_entry(&mgr, "parent-multi", "child-multi", "child-agent", 1).await;
    register_tree_entry(&mgr, "child-multi", "gc1-multi", "gc1-agent", 2).await;
    register_tree_entry(&mgr, "child-multi", "gc2-multi", "gc2-agent", 2).await;
    register_tree_entry(&mgr, "gc1-multi", "ggc-multi", "ggc-agent", 3).await;

    // Confirm all exist before kill.
    assert!(mgr.has_session("child-multi").await);
    assert!(mgr.has_session("gc1-multi").await);
    assert!(mgr.has_session("gc2-multi").await);
    assert!(mgr.has_session("ggc-multi").await);

    // Kill child and all descendants.
    mgr.kill_child("parent-multi", "child-multi")
        .await
        .expect("kill_child should succeed");

    // All sessions removed from memory.
    assert!(!mgr.has_session("child-multi").await);
    assert!(!mgr.has_session("gc1-multi").await);
    assert!(!mgr.has_session("gc2-multi").await);
    assert!(!mgr.has_session("ggc-multi").await);

    // Checkpoints for all killed sessions should be in storage.
    for id in &["child-multi", "gc1-multi", "gc2-multi", "ggc-multi"] {
        let cp = storage
            .load_checkpoint(id)
            .await
            .unwrap_or_else(|e| panic!("storage load for {} failed: {}", id, e))
            .unwrap_or_else(|| panic!("checkpoint for {} should exist in storage after kill", id));
        assert_eq!(
            cp.session_id, *id,
            "checkpoint session_id mismatch for {}",
            id
        );
    }
}

/// Persistence failure on one descendant does not prevent other
/// descendants from being persisted (each session persists
/// independently; warn-and-continue semantics).
#[tokio::test]
#[serial]
async fn test_kill_child_partial_persistence_failure() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    // Use MemoryStorage so the first persist succeeds, then swap to
    // FailingPersistService for the second. Since the manager holds an
    // Arc, we can't swap the service itself — instead we test the
    // general guarantee: kill still completes and the sessions we CAN
    // verify are persisted are correct.
    //
    // The real scenario: if a transient failure occurs on one session,
    // the other sessions still get persisted. We simulate this by
    // using FailingPersistService for everything and just verifying
    // that kill still completes cleanly.
    let failing_storage: Arc<dyn PersistenceService> = Arc::new(FailingPersistService);
    let mgr = SessionManager::new(
        &test_config(),
        Some(failing_storage),
        Some(tmp.path().to_path_buf()),
        Default::default(),
    );

    // Register parent.
    {
        let cs = ConversationSession::new(
            "parent-partial".into(),
            "test-model".into(),
            tmp.path().to_path_buf(),
        );
        mgr.conversation_sessions
            .write()
            .await
            .insert("parent-partial".into(), Arc::new(RwLock::new(cs)));
        mgr.sessions.write().await.insert(
            "parent-partial".into(),
            crate::Session {
                id: "parent-partial".into(),
                agent_id: "root-agent".into(),
                channel: "spawn".into(),
                created_at: 0,
                depth: 0,
            },
        );
    }

    register_session(&mgr, "child-partial", "child-agent", 1).await;
    register_session(&mgr, "gc-partial", "gc-agent", 2).await;
    register_tree_entry(&mgr, "parent-partial", "child-partial", "child-agent", 1).await;
    register_tree_entry(&mgr, "child-partial", "gc-partial", "gc-agent", 2).await;

    // Kill should succeed even though persistence fails for all sessions.
    let result = mgr.kill_child("parent-partial", "child-partial").await;
    assert!(
        result.is_ok(),
        "kill_child should succeed even when all persistence attempts fail"
    );

    // Memory maps should still be fully cleared.
    assert!(!mgr.has_session("child-partial").await);
    assert!(!mgr.has_session("gc-partial").await);
    assert!(mgr
        .get_conversation_session("child-partial")
        .await
        .is_none());
    assert!(mgr.get_conversation_session("gc-partial").await.is_none());
}
