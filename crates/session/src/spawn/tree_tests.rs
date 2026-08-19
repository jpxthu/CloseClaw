//! Unit tests for SpawnTree: mark_child_status and active filtering.

use super::tree::SpawnTree;
use super::types::{ChildSessionInfo, ChildSessionStatus, SpawnMode};

#[test]
fn test_mark_child_status_and_active_count() {
    let mut tree = SpawnTree::new();

    // Register two children
    tree.register_child(
        "parent",
        ChildSessionInfo {
            session_id: "child-1".to_string(),
            parent_session_id: "parent".to_string(),
            agent_id: "agent-a".to_string(),
            depth: 1,
            mode: SpawnMode::Run,
            status: ChildSessionStatus::Active,
            timeout_secs: None,
            timeout_warning_secs: None,
            timeout_notify_interval_ratio: None,
            created_at: std::time::Instant::now(),
        },
    );
    tree.register_child(
        "parent",
        ChildSessionInfo {
            session_id: "child-2".to_string(),
            parent_session_id: "parent".to_string(),
            agent_id: "agent-b".to_string(),
            depth: 1,
            mode: SpawnMode::Session,
            status: ChildSessionStatus::Active,
            timeout_secs: None,
            timeout_warning_secs: None,
            timeout_notify_interval_ratio: None,
            created_at: std::time::Instant::now(),
        },
    );

    // Both active
    let children = tree.list_children("parent");
    let active_count = children
        .iter()
        .filter(|c| c.status == ChildSessionStatus::Active)
        .count();
    assert_eq!(active_count, 2, "both children should be active initially");

    // Mark child-1 as Completed
    let updated = tree.mark_child_status("child-1", ChildSessionStatus::Completed);
    assert!(
        updated,
        "mark_child_status should return true for existing child"
    );

    // Now only child-2 is active
    let children = tree.list_children("parent");
    let active_count = children
        .iter()
        .filter(|c| c.status == ChildSessionStatus::Active)
        .count();
    assert_eq!(
        active_count, 1,
        "only child-2 should be active after child-1 completed"
    );
    assert_eq!(children[0].status, ChildSessionStatus::Completed);
    assert_eq!(children[1].status, ChildSessionStatus::Active);

    // Mark child-2 as Terminated
    tree.mark_child_status("child-2", ChildSessionStatus::Terminated);
    let children = tree.list_children("parent");
    let active_count = children
        .iter()
        .filter(|c| c.status == ChildSessionStatus::Active)
        .count();
    assert_eq!(
        active_count, 0,
        "no children should be active after both marked"
    );

    // mark_child_status for non-existent child returns false
    let not_found = tree.mark_child_status("nonexistent", ChildSessionStatus::Completed);
    assert!(
        !not_found,
        "mark_child_status should return false for unknown child"
    );
}

#[test]
fn test_mark_child_status_completed_then_kill() {
    let mut tree = SpawnTree::new();

    tree.register_child(
        "parent",
        ChildSessionInfo {
            session_id: "child-1".to_string(),
            parent_session_id: "parent".to_string(),
            agent_id: "agent-a".to_string(),
            depth: 1,
            mode: SpawnMode::Run,
            status: ChildSessionStatus::Active,
            timeout_secs: None,
            timeout_warning_secs: None,
            timeout_notify_interval_ratio: None,
            created_at: std::time::Instant::now(),
        },
    );

    // Mark as Completed, then remove (kill)
    tree.mark_child_status("child-1", ChildSessionStatus::Completed);
    assert_eq!(
        tree.list_children("parent")[0].status,
        ChildSessionStatus::Completed
    );

    tree.remove_child("parent", "child-1");
    assert!(
        tree.list_children("parent").is_empty(),
        "completed child removed after kill"
    );
}

#[test]
fn test_mark_child_status_terminate_before_active() {
    let mut tree = SpawnTree::new();

    tree.register_child(
        "parent",
        ChildSessionInfo {
            session_id: "child-1".to_string(),
            parent_session_id: "parent".to_string(),
            agent_id: "agent-a".to_string(),
            depth: 1,
            mode: SpawnMode::Session,
            status: ChildSessionStatus::Active,
            timeout_secs: None,
            timeout_warning_secs: None,
            timeout_notify_interval_ratio: None,
            created_at: std::time::Instant::now(),
        },
    );

    // Terminate directly (not going through Completed first)
    tree.mark_child_status("child-1", ChildSessionStatus::Terminated);
    let children = tree.list_children("parent");
    assert_eq!(children[0].status, ChildSessionStatus::Terminated);
    // Should not be counted as active
    let active = children
        .iter()
        .filter(|c| c.status == ChildSessionStatus::Active)
        .count();
    assert_eq!(active, 0);
}

// ── reclaim_completed tests ──────────────────────────────────────

fn make_child(
    parent_id: &str,
    child_id: &str,
    agent_id: &str,
    status: ChildSessionStatus,
) -> ChildSessionInfo {
    ChildSessionInfo {
        session_id: child_id.to_string(),
        parent_session_id: parent_id.to_string(),
        agent_id: agent_id.to_string(),
        depth: 1,
        mode: SpawnMode::Run,
        status,
        timeout_secs: None,
        timeout_warning_secs: None,
        timeout_notify_interval_ratio: None,
        created_at: std::time::Instant::now(),
    }
}

fn make_child_with_depth(
    parent_id: &str,
    child_id: &str,
    agent_id: &str,
    status: ChildSessionStatus,
    depth: u32,
) -> ChildSessionInfo {
    ChildSessionInfo {
        session_id: child_id.to_string(),
        parent_session_id: parent_id.to_string(),
        agent_id: agent_id.to_string(),
        depth,
        mode: SpawnMode::Run,
        status,
        timeout_secs: None,
        timeout_warning_secs: None,
        timeout_notify_interval_ratio: None,
        created_at: std::time::Instant::now(),
    }
}

#[test]
fn test_reclaim_completed_removes_terminal_child() {
    let mut tree = SpawnTree::new();
    tree.register_child(
        "parent",
        make_child("parent", "c1", "a", ChildSessionStatus::Completed),
    );

    let reclaimed = tree.reclaim_completed("parent");
    assert_eq!(reclaimed, vec!["c1".to_string()]);
    assert!(tree.list_children("parent").is_empty());
}

#[test]
fn test_reclaim_completed_removes_terminated_child() {
    let mut tree = SpawnTree::new();
    tree.register_child(
        "parent",
        make_child("parent", "c1", "a", ChildSessionStatus::Terminated),
    );

    let reclaimed = tree.reclaim_completed("parent");
    assert_eq!(reclaimed, vec!["c1".to_string()]);
}

#[test]
fn test_reclaim_completed_preserves_active_child() {
    let mut tree = SpawnTree::new();
    tree.register_child(
        "parent",
        make_child("parent", "c1", "a", ChildSessionStatus::Completed),
    );
    tree.register_child(
        "parent",
        make_child("parent", "c2", "b", ChildSessionStatus::Active),
    );

    let reclaimed = tree.reclaim_completed("parent");
    assert_eq!(reclaimed, vec!["c1".to_string()]);
    assert_eq!(tree.list_children("parent").len(), 1);
    assert_eq!(tree.list_children("parent")[0].session_id, "c2");
}

#[test]
fn test_reclaim_completed_mixed_terminal_states() {
    let mut tree = SpawnTree::new();
    tree.register_child(
        "parent",
        make_child("parent", "c1", "a", ChildSessionStatus::Completed),
    );
    tree.register_child(
        "parent",
        make_child("parent", "c2", "b", ChildSessionStatus::Terminated),
    );
    tree.register_child(
        "parent",
        make_child("parent", "c3", "c", ChildSessionStatus::Active),
    );

    let mut reclaimed = tree.reclaim_completed("parent");
    reclaimed.sort();
    assert_eq!(reclaimed, vec!["c1".to_string(), "c2".to_string()]);
    assert_eq!(tree.list_children("parent").len(), 1);
    assert_eq!(tree.list_children("parent")[0].session_id, "c3");
}

#[test]
fn test_reclaim_completed_empty_parent() {
    let mut tree = SpawnTree::new();
    let reclaimed = tree.reclaim_completed("nonexistent");
    assert!(reclaimed.is_empty());
}

#[test]
fn test_reclaim_completed_skips_node_with_active_descendant() {
    // parent -> c1 (Completed) -> c2 (Active)
    // c1 should NOT be reclaimed because it has an active descendant
    let mut tree = SpawnTree::new();
    tree.register_child(
        "parent",
        make_child("parent", "c1", "a", ChildSessionStatus::Completed),
    );
    tree.register_child(
        "c1",
        make_child("c1", "c2", "b", ChildSessionStatus::Active),
    );

    let reclaimed = tree.reclaim_completed("parent");
    assert!(
        reclaimed.is_empty(),
        "should not reclaim node with active descendants"
    );
    assert_eq!(tree.list_children("parent").len(), 1);
}

#[test]
fn test_reclaim_completed_removes_node_with_all_terminal_descendants() {
    // parent -> c1 (Completed) -> c2 (Completed)
    // c1 should be reclaimed (its subtree is fully terminal)
    let mut tree = SpawnTree::new();
    tree.register_child(
        "parent",
        make_child("parent", "c1", "a", ChildSessionStatus::Completed),
    );
    tree.register_child(
        "c1",
        make_child("c1", "c2", "b", ChildSessionStatus::Completed),
    );

    let reclaimed = tree.reclaim_completed("parent");
    assert_eq!(reclaimed, vec!["c1".to_string()]);
    assert!(tree.list_children("parent").is_empty());
    // Note: c2 still exists under c1 (orphaned sub-entry).
    // Full recursive cleanup is handled by GC (remove_descendant_entries).
    assert_eq!(tree.list_children("c1").len(), 1);
}

#[test]
fn test_reclaim_completed_idempotent() {
    let mut tree = SpawnTree::new();
    tree.register_child(
        "parent",
        make_child("parent", "c1", "a", ChildSessionStatus::Completed),
    );

    let reclaimed1 = tree.reclaim_completed("parent");
    assert_eq!(reclaimed1, vec!["c1".to_string()]);

    let reclaimed2 = tree.reclaim_completed("parent");
    assert!(reclaimed2.is_empty(), "second call should reclaim nothing");
}

#[test]
fn test_reclaim_completed_three_level_active_deep() {
    // parent -> c1 (Completed) -> c2 (Completed) -> c3 (Active)
    // c1 should NOT be reclaimed because c3 (deep descendant) is active
    let mut tree = SpawnTree::new();
    tree.register_child(
        "parent",
        make_child("parent", "c1", "a", ChildSessionStatus::Completed),
    );
    tree.register_child(
        "c1",
        make_child_with_depth("c1", "c2", "b", ChildSessionStatus::Completed, 2),
    );
    tree.register_child(
        "c2",
        make_child_with_depth("c2", "c3", "c", ChildSessionStatus::Active, 3),
    );

    let reclaimed = tree.reclaim_completed("parent");
    assert!(
        reclaimed.is_empty(),
        "should not reclaim with deep active descendant"
    );
}
