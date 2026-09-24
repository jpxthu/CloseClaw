//! cascade_kill_children / cascade_archive tests: descendant cleanup and
//! deletion ordering across the session tree.

use closeclaw_session::persistence::{PersistenceService, SessionCheckpoint};
use std::sync::Arc;

use crate::sweeper::ArchiveSweeper;

use super::sweeper_test_utils::MemStorage;

// -----------------------------------------------------------------
// Test: cascade_kill_children deletes all descendants
// -----------------------------------------------------------------

#[tokio::test]
async fn test_cascade_kill_children_deletes_descendants() {
    let mem = Arc::new(MemStorage::default());

    // Build a 3-level tree: parent -> child1 -> grandchild
    let mut parent = SessionCheckpoint::new("parent".into());
    parent.parent_session_id = None;
    parent.depth = 0;
    mem.add_checkpoint(parent);

    let mut child1 = SessionCheckpoint::new("child1".into());
    child1.parent_session_id = Some("parent".into());
    child1.depth = 1;
    mem.add_checkpoint(child1);

    let mut grandchild = SessionCheckpoint::new("grandchild".into());
    grandchild.parent_session_id = Some("child1".into());
    grandchild.depth = 2;
    mem.add_checkpoint(grandchild);

    // Run cascade
    let storage: Arc<dyn PersistenceService> = mem.clone() as _;
    ArchiveSweeper::cascade_kill_children(storage.as_ref(), "parent")
        .await
        .unwrap();

    // Both descendants should be deleted
    let deleted = mem.deleted_ids();
    assert!(
        deleted.contains(&"child1".to_string()),
        "cascade must delete child1; actual deleted ids: {deleted:?}"
    );
    assert!(
        deleted.contains(&"grandchild".to_string()),
        "cascade must delete grandchild; actual deleted ids: {deleted:?}"
    );
    // Parent itself should NOT be deleted by cascade_kill_children
    assert!(
        !deleted.contains(&"parent".to_string()),
        "cascade must not delete parent itself; actual deleted ids: {deleted:?}"
    );
}

#[tokio::test]
async fn test_cascade_kill_children_no_children_noop() {
    let mem = Arc::new(MemStorage::default());
    mem.add_checkpoint(SessionCheckpoint::new("leaf".into()));

    let storage: Arc<dyn PersistenceService> = mem.clone() as _;
    ArchiveSweeper::cascade_kill_children(storage.as_ref(), "leaf")
        .await
        .unwrap();

    // No children to delete
    let deleted = mem.deleted_ids();
    assert!(
        deleted.is_empty(),
        "no children: cascade must delete nothing; actual deleted ids: {deleted:?}"
    );
}

#[tokio::test]
async fn test_cascade_kill_children_siblings_only() {
    let mem = Arc::new(MemStorage::default());

    let mut parent = SessionCheckpoint::new("parent".into());
    parent.parent_session_id = None;
    mem.add_checkpoint(parent);

    let mut child_a = SessionCheckpoint::new("child_a".into());
    child_a.parent_session_id = Some("parent".into());
    mem.add_checkpoint(child_a);

    let mut child_b = SessionCheckpoint::new("child_b".into());
    child_b.parent_session_id = Some("parent".into());
    mem.add_checkpoint(child_b);

    // Unrelated child under a different parent
    let mut other_child = SessionCheckpoint::new("other_child".into());
    other_child.parent_session_id = Some("other_parent".into());
    mem.add_checkpoint(other_child);

    let storage: Arc<dyn PersistenceService> = mem.clone() as _;
    ArchiveSweeper::cascade_kill_children(storage.as_ref(), "parent")
        .await
        .unwrap();

    let deleted = mem.deleted_ids();
    assert!(
        deleted.contains(&"child_a".to_string()),
        "cascade must delete child_a under parent; actual deleted ids: {deleted:?}"
    );
    assert!(
        deleted.contains(&"child_b".to_string()),
        "cascade must delete child_b under parent; actual deleted ids: {deleted:?}"
    );
    // other_child should NOT be deleted
    assert!(
        !deleted.contains(&"other_child".to_string()),
        "cascade must not delete other_child (other parent); actual deleted ids: {deleted:?}"
    );
}

// -----------------------------------------------------------------
// Test: cascade_archive kills children then archives parent
// -----------------------------------------------------------------

#[tokio::test]
async fn test_cascade_archive_kills_children_then_archives_parent() {
    let mem = Arc::new(MemStorage::default());

    let mut parent = SessionCheckpoint::new("parent-archive".into());
    parent.parent_session_id = None;
    mem.add_checkpoint(parent);

    let mut child = SessionCheckpoint::new("child-archive".into());
    child.parent_session_id = Some("parent-archive".into());
    mem.add_checkpoint(child);

    let storage: Arc<dyn PersistenceService> = mem.clone() as _;
    ArchiveSweeper::cascade_archive_impl(storage, "parent-archive".into())
        .await
        .unwrap();

    // Child should be deleted
    let deleted = mem.deleted_ids();
    assert!(
        deleted.contains(&"child-archive".to_string()),
        "cascade_archive must delete child-archive; actual deleted ids: {deleted:?}"
    );

    // Parent should be archived
    let archive_called = mem.archive_called.lock().unwrap();
    assert!(
        archive_called.contains(&"parent-archive".into()),
        "cascade_archive must archive parent-archive; actual archive calls: {archive_called:?}"
    );
}

#[tokio::test]
async fn test_cascade_archive_no_children_archives_parent() {
    let mem = Arc::new(MemStorage::default());
    mem.add_checkpoint(SessionCheckpoint::new("solo-parent".into()));

    let storage: Arc<dyn PersistenceService> = mem.clone() as _;
    ArchiveSweeper::cascade_archive_impl(storage, "solo-parent".into())
        .await
        .unwrap();

    // No children deleted
    let deleted = mem.deleted_ids();
    assert!(
        deleted.is_empty(),
        "no children: cascade_archive must delete nothing; actual deleted ids: {deleted:?}"
    );

    // Parent should be archived
    let archive_called = mem.archive_called.lock().unwrap();
    assert!(
        archive_called.contains(&"solo-parent".into()),
        "cascade_archive must archive solo-parent; actual archive calls: {archive_called:?}"
    );
}

// -----------------------------------------------------------------
// Test: multi-branch tree — leaf nodes deleted before branch nodes
// -----------------------------------------------------------------

#[tokio::test]
async fn test_cascade_kill_children_multi_branch_tree() {
    let mem = Arc::new(MemStorage::default());

    // Build a multi-branch tree:
    //       root
    //      /    \
    //    child_a  child_b
    //    /    \        \
    //  gc_a1  gc_a2    gc_b1
    let mut root = SessionCheckpoint::new("root".into());
    root.parent_session_id = None;
    root.depth = 0;
    mem.add_checkpoint(root);

    let mut child_a = SessionCheckpoint::new("child_a".into());
    child_a.parent_session_id = Some("root".into());
    child_a.depth = 1;
    mem.add_checkpoint(child_a);

    let mut child_b = SessionCheckpoint::new("child_b".into());
    child_b.parent_session_id = Some("root".into());
    child_b.depth = 1;
    mem.add_checkpoint(child_b);

    let mut gc_a1 = SessionCheckpoint::new("gc_a1".into());
    gc_a1.parent_session_id = Some("child_a".into());
    gc_a1.depth = 2;
    mem.add_checkpoint(gc_a1);

    let mut gc_a2 = SessionCheckpoint::new("gc_a2".into());
    gc_a2.parent_session_id = Some("child_a".into());
    gc_a2.depth = 2;
    mem.add_checkpoint(gc_a2);

    let mut gc_b1 = SessionCheckpoint::new("gc_b1".into());
    gc_b1.parent_session_id = Some("child_b".into());
    gc_b1.depth = 2;
    mem.add_checkpoint(gc_b1);

    // Run cascade
    let storage: Arc<dyn PersistenceService> = mem.clone() as _;
    ArchiveSweeper::cascade_kill_children(storage.as_ref(), "root")
        .await
        .unwrap();

    // All 5 descendants should be deleted
    let deleted = mem.deleted_ids();
    assert!(
        deleted.contains(&"child_a".to_string()),
        "multi-branch cascade must delete child_a; actual deleted ids: {deleted:?}"
    );
    assert!(
        deleted.contains(&"child_b".to_string()),
        "multi-branch cascade must delete child_b; actual deleted ids: {deleted:?}"
    );
    assert!(
        deleted.contains(&"gc_a1".to_string()),
        "multi-branch cascade must delete gc_a1; actual deleted ids: {deleted:?}"
    );
    assert!(
        deleted.contains(&"gc_a2".to_string()),
        "multi-branch cascade must delete gc_a2; actual deleted ids: {deleted:?}"
    );
    assert!(
        deleted.contains(&"gc_b1".to_string()),
        "multi-branch cascade must delete gc_b1; actual deleted ids: {deleted:?}"
    );
    // Root itself should NOT be deleted
    assert!(
        !deleted.contains(&"root".to_string()),
        "multi-branch cascade must not delete root; actual deleted ids: {deleted:?}"
    );
    assert_eq!(deleted.len(), 5);

    // Verify deletion order: all depth-2 nodes before depth-1 nodes
    let pos_gc_a1 = deleted.iter().position(|id| id == "gc_a1").unwrap();
    let pos_gc_a2 = deleted.iter().position(|id| id == "gc_a2").unwrap();
    let pos_gc_b1 = deleted.iter().position(|id| id == "gc_b1").unwrap();
    let pos_child_a = deleted.iter().position(|id| id == "child_a").unwrap();
    let pos_child_b = deleted.iter().position(|id| id == "child_b").unwrap();

    assert!(
        pos_gc_a1 < pos_child_a,
        "gc_a1 must be deleted before child_a"
    );
    assert!(
        pos_gc_a2 < pos_child_a,
        "gc_a2 must be deleted before child_a"
    );
    assert!(
        pos_gc_b1 < pos_child_b,
        "gc_b1 must be deleted before child_b"
    );
}
