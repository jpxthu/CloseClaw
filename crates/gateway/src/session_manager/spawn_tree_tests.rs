//! Unit tests for `SpawnTree` query interfaces.
//!
//! Tests cover the three design-doc query APIs:
//! - `list_children`: with children / without children / empty session_id
//! - `list_descendants`: multi-level nesting / single level / no descendants
//! - `get_parent`: has parent / root node (no parent)

use super::spawn::{ChildSessionInfo, SpawnMode, SpawnTree};

// ── Helpers ──────────────────────────────────────────────────────────────

fn info(session_id: &str, parent_session_id: &str, depth: u32) -> ChildSessionInfo {
    ChildSessionInfo {
        session_id: session_id.to_string(),
        parent_session_id: parent_session_id.to_string(),
        agent_id: "test-agent".to_string(),
        depth,
        mode: SpawnMode::Run,
    }
}

// ── list_children ────────────────────────────────────────────────────────

#[test]
fn test_list_children_with_children() {
    let mut tree = SpawnTree::new();
    tree.register_child("root", info("child-a", "root", 1));
    tree.register_child("root", info("child-b", "root", 1));

    let children = tree.list_children("root");
    assert_eq!(children.len(), 2);
    let ids: Vec<&str> = children.iter().map(|c| c.session_id.as_str()).collect();
    assert!(ids.contains(&"child-a"));
    assert!(ids.contains(&"child-b"));
}

#[test]
fn test_list_children_without_children() {
    let tree = SpawnTree::new();
    let children = tree.list_children("nonexistent");
    assert!(children.is_empty());
}

#[test]
fn test_list_children_empty_session_id() {
    let mut tree = SpawnTree::new();
    tree.register_child("root", info("child-a", "root", 1));

    let children = tree.list_children("");
    assert!(children.is_empty());
}

#[test]
fn test_list_children_returns_empty_vec_for_unknown_parent() {
    let mut tree = SpawnTree::new();
    tree.register_child("parent-a", info("child-x", "parent-a", 1));

    let children = tree.list_children("parent-b");
    assert!(children.is_empty());
}

// ── list_descendants ─────────────────────────────────────────────────────

#[test]
fn test_list_descendants_no_descendants() {
    let tree = SpawnTree::new();
    let descendants = tree.list_descendants("root");
    assert!(descendants.is_empty());
}

#[test]
fn test_list_descendants_single_level() {
    let mut tree = SpawnTree::new();
    tree.register_child("root", info("a", "root", 1));
    tree.register_child("root", info("b", "root", 1));

    let descendants = tree.list_descendants("root");
    // BFS reversed → deepest first, but all at depth 1 so order is
    // reversed insertion: b, a
    assert_eq!(descendants, vec!["b".to_string(), "a".to_string()]);
}

#[test]
fn test_list_descendants_multi_level() {
    // root → a, b
    // a → a1, a2
    // a1 → a1a
    //
    // BFS order: a, b, a1, a2, a1a
    // Reversed (deepest first): a1a, a2, a1, b, a
    let mut tree = SpawnTree::new();
    tree.register_child("root", info("a", "root", 1));
    tree.register_child("root", info("b", "root", 1));
    tree.register_child("a", info("a1", "a", 2));
    tree.register_child("a", info("a2", "a", 2));
    tree.register_child("a1", info("a1a", "a1", 3));

    let descendants = tree.list_descendants("root");
    assert_eq!(
        descendants,
        vec![
            "a1a".to_string(),
            "a2".to_string(),
            "a1".to_string(),
            "b".to_string(),
            "a".to_string(),
        ]
    );
}

#[test]
fn test_list_descendants_unknown_session_returns_empty() {
    let mut tree = SpawnTree::new();
    tree.register_child("root", info("a", "root", 1));

    let descendants = tree.list_descendants("nonexistent");
    assert!(descendants.is_empty());
}

#[test]
fn test_list_descendants_empty_session_id() {
    let mut tree = SpawnTree::new();
    tree.register_child("root", info("a", "root", 1));

    let descendants = tree.list_descendants("");
    assert!(descendants.is_empty());
}

#[test]
fn test_list_descendants_middle_node() {
    // root → a → a1 → a1a
    let mut tree = SpawnTree::new();
    tree.register_child("root", info("a", "root", 1));
    tree.register_child("a", info("a1", "a", 2));
    tree.register_child("a1", info("a1a", "a1", 3));

    let descendants = tree.list_descendants("a");
    // BFS from "a": a1, a1a → reversed: a1a, a1
    assert_eq!(descendants, vec!["a1a".to_string(), "a1".to_string()]);
}

// ── get_parent ───────────────────────────────────────────────────────────

#[test]
fn test_get_parent_has_parent() {
    let mut tree = SpawnTree::new();
    tree.register_child("root", info("child-a", "root", 1));

    let parent = tree.get_parent("child-a");
    assert_eq!(parent.as_deref(), Some("root"));
}

#[test]
fn test_get_parent_root_node_no_parent() {
    let mut tree = SpawnTree::new();
    tree.register_child("root", info("child-a", "root", 1));

    // "root" is a top-level session — not registered as a child of
    // anything, so get_parent should return None.
    let parent = tree.get_parent("root");
    assert_eq!(parent, None);
}

#[test]
fn test_get_parent_unknown_session() {
    let tree = SpawnTree::new();
    let parent = tree.get_parent("nonexistent");
    assert_eq!(parent, None);
}

#[test]
fn test_get_parent_empty_session_id() {
    let mut tree = SpawnTree::new();
    tree.register_child("root", info("child-a", "root", 1));

    let parent = tree.get_parent("");
    assert_eq!(parent, None);
}

#[test]
fn test_get_parent_multi_level() {
    let mut tree = SpawnTree::new();
    tree.register_child("root", info("a", "root", 1));
    tree.register_child("a", info("a1", "a", 2));
    tree.register_child("a1", info("a1a", "a1", 3));

    assert_eq!(tree.get_parent("a").as_deref(), Some("root"));
    assert_eq!(tree.get_parent("a1").as_deref(), Some("a"));
    assert_eq!(tree.get_parent("a1a").as_deref(), Some("a1"));
    assert_eq!(tree.get_parent("root"), None);
}
