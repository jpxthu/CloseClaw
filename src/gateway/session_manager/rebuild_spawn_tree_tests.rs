//! Tests for `SessionManager::rebuild_spawn_tree`.
//!
//! These tests verify that `rebuild_spawn_tree` correctly reconstructs the
//! in-memory children table from persisted `SessionCheckpoint`s.

use super::tests::{make_test_mgr, test_config};
use super::SessionManager;
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::{PersistenceService, SessionCheckpoint};
use crate::session::ReasoningLevel;
use std::sync::Arc;

/// Helper: create a `SessionManager` with the given `MemoryStorage` and
/// no workspace (avoids filesystem dependencies).
fn make_mgr_with_storage(
    storage: Arc<crate::session::storage::memory::MemoryStorage>,
) -> SessionManager {
    SessionManager::new(
        &test_config(),
        Some(storage),
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    )
}

/// `rebuild_spawn_tree` basic: parent and child checkpoints stored;
/// after rebuild the children table should contain the child under
/// the parent with the correct fields.
#[tokio::test]
async fn test_rebuild_spawn_tree_basic() {
    let storage = Arc::new(crate::session::storage::memory::MemoryStorage::new());

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

    // children table should have one entry under parent-1.
    let count = mgr.count_active_children("parent-1").await;
    assert_eq!(count, 1, "parent-1 should have exactly 1 child");

    let children = mgr.children.read().await;
    let list = children.get("parent-1").unwrap();
    assert_eq!(list[0].session_id, "child-1");
    assert_eq!(list[0].parent_session_id, "parent-1");
    assert_eq!(list[0].agent_id, "child-agent");
    assert_eq!(list[0].depth, 1);
}

/// `rebuild_spawn_tree` orphan: child's parent does not exist in storage.
/// The child should NOT be registered in the children table (degraded to root).
#[tokio::test]
async fn test_rebuild_spawn_tree_orphan() {
    let storage = Arc::new(crate::session::storage::memory::MemoryStorage::new());

    // Only child checkpoint, no parent.
    let mut child_cp = SessionCheckpoint::new("orphan-child".to_string());
    child_cp.depth = 2;
    child_cp.agent_id = Some("orphan-agent".to_string());
    child_cp.parent_session_id = Some("nonexistent-parent".to_string());
    storage.save_checkpoint(&child_cp).await.unwrap();

    let mgr = make_mgr_with_storage(storage);
    mgr.rebuild_spawn_tree().await.unwrap();

    // children table should be empty — orphan degraded to root.
    let children = mgr.children.read().await;
    assert!(
        children.is_empty(),
        "orphan child should not appear in children table"
    );
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
