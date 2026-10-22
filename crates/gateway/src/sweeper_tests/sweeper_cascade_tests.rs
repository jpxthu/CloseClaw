//! cascade_kill_children / cascade_archive tests: descendant cleanup and
//! deletion ordering across the session tree.

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
    mem.add_root("parent");
    mem.add_child("child1", "parent", 1);
    mem.add_child("grandchild", "child1", 2);

    // Run cascade
    let storage = mem.storage_arc();
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
    mem.add_root("leaf");

    let storage = mem.storage_arc();
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

    mem.add_root("parent");
    mem.add_child("child_a", "parent", 0);
    mem.add_child("child_b", "parent", 0);

    // Unrelated child under a different parent
    mem.add_child("other_child", "other_parent", 0);

    let storage = mem.storage_arc();
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

    mem.add_root("parent-archive");
    mem.add_child("child-archive", "parent-archive", 0);

    let storage = mem.storage_arc();
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
    mem.add_root("solo-parent");

    let storage = mem.storage_arc();
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
    // Build a multi-branch tree:
    //       root
    //      /    \
    //    child_a  child_b
    //    /    \        \
    //  gc_a1  gc_a2    gc_b1
    mem.add_root("root");
    mem.add_child("child_a", "root", 1);
    mem.add_child("child_b", "root", 1);
    mem.add_child("gc_a1", "child_a", 2);
    mem.add_child("gc_a2", "child_a", 2);
    mem.add_child("gc_b1", "child_b", 2);

    // Run cascade
    let storage = mem.storage_arc();
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
