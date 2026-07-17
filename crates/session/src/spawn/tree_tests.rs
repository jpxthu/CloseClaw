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
