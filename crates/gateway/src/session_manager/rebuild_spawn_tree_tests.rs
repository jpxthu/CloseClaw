//! Tests for `SessionManager::rebuild_spawn_tree`.
//!
//! These tests verify that `rebuild_spawn_tree` correctly reconstructs the
//! in-memory children table from persisted `SessionCheckpoint`s.

use super::tests::{make_test_mgr, test_config};
use super::SessionManager;
use crate::Session;
use chrono::Utc;
use closeclaw_session::persistence::ReasoningLevel;
use closeclaw_session::persistence::{PersistenceError, PersistenceService, SessionCheckpoint};
use closeclaw_session::spawn::SpawnMode;
use std::sync::Arc;

/// Helper: create a `SessionManager` with the given `MemoryStorage` and
/// no workspace (avoids filesystem dependencies).
fn make_mgr_with_storage(
    storage: Arc<closeclaw_session::storage::memory::MemoryStorage>,
) -> SessionManager {
    SessionManager::new(
        &test_config(),
        Some(storage),
        None,
        ReasoningLevel::default(),
    )
}

/// Helper: create a `SessionManager` with any `PersistenceService` impl.
fn make_mgr_with_persistence(storage: Arc<dyn PersistenceService>) -> SessionManager {
    SessionManager::new(
        &test_config(),
        Some(storage),
        None,
        ReasoningLevel::default(),
    )
}

/// `rebuild_spawn_tree` basic: parent and child checkpoints stored;
/// after rebuild the children table should contain the child under
/// the parent with the correct fields.
#[tokio::test]
async fn test_rebuild_spawn_tree_basic() {
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());

    // Parent checkpoint (root, no parent_session_id).
    let mut parent_cp = SessionCheckpoint::new("parent-1".to_string());
    parent_cp.depth = 0;
    parent_cp.agent_id = Some("parent-agent".to_string());
    parent_cp.parent_session_id = None;
    storage.save_checkpoint(&parent_cp).await.unwrap();

    // Child checkpoint (parent = parent-1).
    let mut child_cp = SessionCheckpoint::new("child-1".to_string());
    child_cp.depth = 1;
    child_cp.agent_id = Some("child-agent".to_string());
    child_cp.parent_session_id = Some("parent-1".to_string());
    storage.save_checkpoint(&child_cp).await.unwrap();

    let mgr = make_mgr_with_storage(storage);
    mgr.rebuild_spawn_tree().await.unwrap();

    // Register child-1 in conversation_sessions so count_active_children
    // (which checks session liveness) counts it correctly.
    let child_cs = closeclaw_session::llm_session::ConversationSession::new(
        "child-1".to_string(),
        "test-model".to_string(),
        std::path::PathBuf::from("/tmp"),
    );
    mgr.conversation_sessions.write().await.insert(
        "child-1".to_string(),
        std::sync::Arc::new(tokio::sync::RwLock::new(child_cs)),
    );

    // children table should have one entry under parent-1.
    let count = mgr.count_active_children("parent-1").await;
    assert_eq!(count, 1, "parent-1 should have exactly 1 child");

    let children = mgr.children.read().await;
    let list = children.list_children("parent-1");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].session_id, "child-1");
    assert_eq!(list[0].parent_session_id, "parent-1");
    assert_eq!(list[0].agent_id, "child-agent");
    assert_eq!(list[0].depth, 1);
}

/// `rebuild_spawn_tree` orphan: child's parent does not exist in storage.
/// The child should NOT be registered in the children table (degraded to root)
/// and its depth should be reset to 0 in the sessions map.
#[tokio::test]
async fn test_rebuild_spawn_tree_orphan() {
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());

    // Only child checkpoint, no parent.
    let mut child_cp = SessionCheckpoint::new("orphan-child".to_string());
    child_cp.depth = 2;
    child_cp.agent_id = Some("orphan-agent".to_string());
    child_cp.parent_session_id = Some("nonexistent-parent".to_string());
    storage.save_checkpoint(&child_cp).await.unwrap();

    let mgr = make_mgr_with_storage(storage);

    // Pre-populate the session in the sessions map with depth=2
    // so we can verify it gets reset to 0 after rebuild.
    mgr.sessions.write().await.insert(
        "orphan-child".to_string(),
        Session {
            id: "orphan-child".to_string(),
            agent_id: "orphan-agent".to_string(),
            channel: "spawn".to_string(),
            created_at: Utc::now().timestamp(),
            depth: 2,
        },
    );

    mgr.rebuild_spawn_tree().await.unwrap();

    // children table should be empty — orphan degraded to root.
    let children = mgr.children.read().await;
    assert!(
        children.is_empty(),
        "orphan child should not appear in children table"
    );

    // Depth should be reset to 0 (degraded to root).
    let sessions = mgr.sessions.read().await;
    let orphan = sessions
        .get("orphan-child")
        .expect("orphan session should exist");
    assert_eq!(orphan.depth, 0, "orphan depth should be reset to 0");
}

/// `rebuild_spawn_tree` no storage: returns Ok and children table stays empty.
#[tokio::test]
async fn test_rebuild_spawn_tree_no_storage() {
    let mgr = make_test_mgr(None); // no storage
    mgr.rebuild_spawn_tree().await.unwrap();

    let children = mgr.children.read().await;
    assert!(
        children.is_empty(),
        "children table should be empty without storage"
    );
}

/// Mock persistence service that returns an error from `load_checkpoint`.
struct FailingCheckpointStorage;

#[async_trait::async_trait]
impl PersistenceService for FailingCheckpointStorage {
    async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            "injected failure",
        )))
    }
    async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec!["session-a".to_string()])
    }
    async fn restore_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            "injected failure",
        )))
    }
    async fn archive_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn purge_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn invalidate_session(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_idle_sessions_for_agent(
        &self,
        _: &str,
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_expired_archived_sessions_for_agent(
        &self,
        _: &str,
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
}

/// `rebuild_spawn_tree` checkpoint error: `load_checkpoint` returns `Err`
/// for a session — should log a warning and skip that session without
/// panicking.
#[tokio::test]
async fn test_rebuild_spawn_tree_checkpoint_error() {
    let storage = Arc::new(FailingCheckpointStorage);
    let mgr = make_mgr_with_persistence(storage);
    // Should complete successfully — errors are logged and skipped.
    mgr.rebuild_spawn_tree().await.unwrap();

    let children = mgr.children.read().await;
    assert!(
        children.is_empty(),
        "children table should be empty when checkpoint loading fails"
    );
}

// ── spawn_mode reconstruction tests ───────────────────────────────────
// Design doc (spawn-tree.md): mode is not persisted to checkpoint and
// defaults to Session on rebuild. All tests verify this semantic.

/// Old checkpoint with spawn_mode serialized as JSON residual key:
/// mode defaults to Session (field no longer read).
#[tokio::test]
async fn test_rebuild_spawn_tree_spawn_mode_residual_json_key() {
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());

    let mut parent_cp = SessionCheckpoint::new("parent-1".to_string());
    parent_cp.depth = 0;
    parent_cp.parent_session_id = None;
    storage.save_checkpoint(&parent_cp).await.unwrap();

    // Simulate old checkpoint JSON with residual "spawn_mode" key.
    // serde ignores unknown fields by default, so the key is silently
    // dropped and mode defaults to Session.
    let raw = serde_json::json!({
        "session_id": "child-1",
        "depth": 1,
        "agent_id": "agent-a",
        "parent_session_id": "parent-1",
        "spawn_mode": "run",
        "created_at": "2025-01-01T00:00:00Z",
        "updated_at": "2025-01-01T00:00:00Z",
        "ttl_seconds": 604800,
        "status": "active",
        "reasoning_mode": "direct",
        "reasoning_level": "low",
        "dreaming_status": "completed",
        "message_count": 0,
        "session_mode": "normal",
        "outbound_pending": [],
        "system_appends": [],
        "pending_operations": [],
        "pending_tool_failures": [],
        "progress_tool_calls": [],
        "approval_tool_calls": [],
        "plan_references": [],
        "pending_messages": [],
        "snapshot_metas": [],
        "mined": false,
        "mode_state": { "current_step": 0, "total_steps": 0, "step_messages": [], "is_complete": false },
        "verbosity_level": "full"
    });
    let child_cp: SessionCheckpoint = serde_json::from_value(raw).unwrap();
    storage.save_checkpoint(&child_cp).await.unwrap();

    let mgr = make_mgr_with_storage(storage);
    mgr.rebuild_spawn_tree().await.unwrap();

    let children = mgr.children.read().await;
    let list = children.list_children("parent-1");
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0].mode,
        SpawnMode::Session,
        "residual spawn_mode JSON key should not affect rebuild — defaults to Session"
    );
}

/// spawn_mode absent (new checkpoint) → defaults to Session
#[tokio::test]
async fn test_rebuild_spawn_tree_spawn_mode_defaults_to_session() {
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());

    let mut parent_cp = SessionCheckpoint::new("parent-1".to_string());
    parent_cp.depth = 0;
    parent_cp.parent_session_id = None;
    storage.save_checkpoint(&parent_cp).await.unwrap();

    let mut child_cp = SessionCheckpoint::new("child-1".to_string());
    child_cp.depth = 1;
    child_cp.agent_id = Some("agent-a".to_string());
    child_cp.parent_session_id = Some("parent-1".to_string());
    storage.save_checkpoint(&child_cp).await.unwrap();

    let mgr = make_mgr_with_storage(storage);
    mgr.rebuild_spawn_tree().await.unwrap();

    let children = mgr.children.read().await;
    let list = children.list_children("parent-1");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].mode, SpawnMode::Session);
}

/// Orphan downgrade: rebuild_spawn_tree should persist effective_max_spawn_depth=None
/// for demoted orphan checkpoints.
#[tokio::test]
async fn test_rebuild_spawn_tree_orphan_effective_max_spawn_depth_reset() {
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());
    let mut child_cp = SessionCheckpoint::new("orphan-budget".to_string());
    child_cp.depth = 2;
    child_cp.agent_id = Some("orphan-agent".to_string());
    child_cp.parent_session_id = Some("nonexistent-parent".to_string());
    child_cp.effective_max_spawn_depth = Some(1);
    storage.save_checkpoint(&child_cp).await.unwrap();
    let mgr = make_mgr_with_storage(storage.clone());
    mgr.rebuild_spawn_tree().await.unwrap();
    let loaded = storage
        .load_checkpoint("orphan-budget")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded.effective_max_spawn_depth, None,
        "demoted orphan checkpoint should have effective_max_spawn_depth reset to None"
    );
    assert_eq!(loaded.depth, 0, "demoted orphan depth should be reset to 0");
}

/// Mixed scenario: multiple children — all default to Session
/// (mode is not persisted to checkpoint per design doc).
#[tokio::test]
async fn test_rebuild_spawn_tree_all_children_default_session() {
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());

    let mut parent_cp = SessionCheckpoint::new("parent-1".to_string());
    parent_cp.depth = 0;
    parent_cp.parent_session_id = None;
    storage.save_checkpoint(&parent_cp).await.unwrap();

    let mut child_a = SessionCheckpoint::new("child-A".to_string());
    child_a.depth = 1;
    child_a.agent_id = Some("agent-a".to_string());
    child_a.parent_session_id = Some("parent-1".to_string());
    storage.save_checkpoint(&child_a).await.unwrap();

    let mut child_b = SessionCheckpoint::new("child-B".to_string());
    child_b.depth = 1;
    child_b.agent_id = Some("agent-b".to_string());
    child_b.parent_session_id = Some("parent-1".to_string());
    storage.save_checkpoint(&child_b).await.unwrap();

    let mgr = make_mgr_with_storage(storage);
    mgr.rebuild_spawn_tree().await.unwrap();

    let children = mgr.children.read().await;
    let list = children.list_children("parent-1");
    assert_eq!(list.len(), 2, "should have 2 children under parent-1");

    for info in &list {
        assert_eq!(
            info.mode,
            SpawnMode::Session,
            "all children should default to Session mode on rebuild"
        );
    }
}
