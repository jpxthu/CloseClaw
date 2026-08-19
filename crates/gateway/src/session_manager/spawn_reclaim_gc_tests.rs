//! Tests for Step 1.3: spawn_reclaim_gc sweep.
//!
//! Verifies that `sweep_spawn_tree_reclaim` correctly reclaims:
//! 1. Terminal滞留 nodes under active parents.
//! 2. Orphaned children whose parent session is gone.
//! 3. Does not touch normal active trees.

use super::spawn::ChildSessionInfo;
use super::spawn::ChildSessionStatus;
use super::spawn::SpawnMode;
use super::spawn_reclaim_gc::sweep_spawn_tree_reclaim;
use super::tests::make_test_mgr;
use crate::Session;
use chrono::Utc;
use closeclaw_session::llm_session::ConversationSession;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// Register a parent session in the `sessions` map only (no
/// ConversationSession). Used by tests that only need the parent
/// to appear active for GC's condition ② check.
async fn setup_parent_sessions_only(mgr: &super::SessionManager, parent_id: &str) {
    mgr.sessions.write().await.insert(
        parent_id.to_string(),
        Session {
            id: parent_id.to_string(),
            agent_id: "parent-agent".to_string(),
            channel: "feishu".to_string(),
            created_at: Utc::now().timestamp(),
            depth: 0,
        },
    );
}

/// Register a parent session with both `sessions` map and
/// `conversation_sessions` (for tests that need full setup).
async fn setup_parent_full(mgr: &super::SessionManager, parent_id: &str) {
    setup_parent_sessions_only(mgr, parent_id).await;
    let cs = Arc::new(RwLock::new(ConversationSession::new(
        parent_id.to_string(),
        "test-model".to_string(),
        std::path::PathBuf::from("/tmp"),
    )));
    mgr.conversation_sessions
        .write()
        .await
        .insert(parent_id.to_string(), cs);
}

/// Register a child node directly in the SpawnTree.
async fn register_child_in_tree(
    mgr: &super::SessionManager,
    parent_id: &str,
    child_id: &str,
    agent_id: &str,
    status: ChildSessionStatus,
) {
    mgr.children.write().await.register_child(
        parent_id,
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
            created_at: Instant::now(),
        },
    );
}

// ── Condition ①: terminal滞留 under active parent ──────────────────────────

/// Terminal children (Completed/Terminated) under an active parent
/// should be reclaimed by the GC sweep.
#[tokio::test]
async fn test_gc_reclaims_terminal_nodes_under_active_parent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    setup_parent_full(&mgr, "parent-active").await;

    register_child_in_tree(
        &mgr,
        "parent-active",
        "child-completed",
        "agent-a",
        ChildSessionStatus::Completed,
    )
    .await;
    register_child_in_tree(
        &mgr,
        "parent-active",
        "child-terminated",
        "agent-b",
        ChildSessionStatus::Terminated,
    )
    .await;
    register_child_in_tree(
        &mgr,
        "parent-active",
        "child-active",
        "agent-c",
        ChildSessionStatus::Active,
    )
    .await;

    sweep_spawn_tree_reclaim(&mgr).await;

    // Terminal nodes should be removed.
    assert!(
        mgr.children
            .read()
            .await
            .find_child("child-completed")
            .is_none(),
        "Completed child should be reclaimed"
    );
    assert!(
        mgr.children
            .read()
            .await
            .find_child("child-terminated")
            .is_none(),
        "Terminated child should be reclaimed"
    );
    // Active child should remain.
    assert!(
        mgr.children
            .read()
            .await
            .find_child("child-active")
            .is_some(),
        "Active child should NOT be reclaimed"
    );
}

// ── Condition ②: orphaned parent (parent gone) ─────────────────────────────

/// When the parent session is gone from `sessions` map, all children
/// (both active and terminal) should be reclaimed.
#[tokio::test]
async fn test_gc_reclaims_orphaned_children() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    // Parent is in tree but NOT in sessions map (ended/archived).
    register_child_in_tree(
        &mgr,
        "parent-gone",
        "orphan-1",
        "agent-a",
        ChildSessionStatus::Completed,
    )
    .await;
    register_child_in_tree(
        &mgr,
        "parent-gone",
        "orphan-2",
        "agent-b",
        ChildSessionStatus::Active,
    )
    .await;

    sweep_spawn_tree_reclaim(&mgr).await;

    // Both children should be reclaimed (parent is gone).
    assert!(
        mgr.children.read().await.find_child("orphan-1").is_none(),
        "orphan-1 should be reclaimed"
    );
    assert!(
        mgr.children.read().await.find_child("orphan-2").is_none(),
        "orphan-2 should be reclaimed"
    );
    // Parent entry should be removed from tree.
    assert!(
        mgr.children
            .read()
            .await
            .list_children("parent-gone")
            .is_empty(),
        "parent-gone entry should be empty after reclaim"
    );
}

// ── Mixed: multiple parents with different states ───────────────────────────

/// GC handles a mix of active parents (condition ①) and orphaned
/// parents (condition ②) in a single sweep.
#[tokio::test]
async fn test_gc_mixed_parents() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));

    // Parent 1: active — has a terminal child to reclaim.
    setup_parent_full(&mgr, "parent-1").await;
    register_child_in_tree(
        &mgr,
        "parent-1",
        "child-1a",
        "agent-a",
        ChildSessionStatus::Completed,
    )
    .await;
    register_child_in_tree(
        &mgr,
        "parent-1",
        "child-1b",
        "agent-b",
        ChildSessionStatus::Active,
    )
    .await;

    // Parent 2: gone — all children should be reclaimed.
    register_child_in_tree(
        &mgr,
        "parent-2",
        "child-2a",
        "agent-c",
        ChildSessionStatus::Active,
    )
    .await;
    register_child_in_tree(
        &mgr,
        "parent-2",
        "child-2b",
        "agent-d",
        ChildSessionStatus::Completed,
    )
    .await;

    sweep_spawn_tree_reclaim(&mgr).await;

    // Parent 1: terminal reclaimed, active preserved.
    assert!(
        mgr.children.read().await.find_child("child-1a").is_none(),
        "child-1a (Completed) should be reclaimed"
    );
    assert!(
        mgr.children.read().await.find_child("child-1b").is_some(),
        "child-1b (Active) should remain"
    );

    // Parent 2: all reclaimed (orphaned).
    assert!(
        mgr.children.read().await.find_child("child-2a").is_none(),
        "child-2a should be reclaimed (orphan)"
    );
    assert!(
        mgr.children.read().await.find_child("child-2b").is_none(),
        "child-2b should be reclaimed (orphan)"
    );
}

// ── Normal active tree: no reclaim ──────────────────────────────────────────

/// When all children are active under an active parent, GC should
/// not touch anything.
#[tokio::test]
async fn test_gc_does_not_touch_active_tree() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    setup_parent_full(&mgr, "parent-healthy").await;

    register_child_in_tree(
        &mgr,
        "parent-healthy",
        "child-h1",
        "agent-a",
        ChildSessionStatus::Active,
    )
    .await;
    register_child_in_tree(
        &mgr,
        "parent-healthy",
        "child-h2",
        "agent-b",
        ChildSessionStatus::Active,
    )
    .await;

    sweep_spawn_tree_reclaim(&mgr).await;

    assert!(
        mgr.children.read().await.find_child("child-h1").is_some(),
        "child-h1 should remain"
    );
    assert!(
        mgr.children.read().await.find_child("child-h2").is_some(),
        "child-h2 should remain"
    );
}

// ── Empty tree: no-op ───────────────────────────────────────────────────────

/// Empty spawn tree produces no errors and no side effects.
#[tokio::test]
async fn test_gc_empty_tree_noop() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    sweep_spawn_tree_reclaim(&mgr).await;
    // No panic, no children to reclaim.
}

// ── Orphan with nested children ─────────────────────────────────────────────

/// Orphaned parent with nested child relationships: all descendants
/// should be reclaimed via `remove_descendant_entries`.
#[tokio::test]
async fn test_gc_reclaims_nested_orphaned_children() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));

    // parent-gone → child-a → grandchild (nested hierarchy)
    mgr.children.write().await.register_child(
        "parent-gone",
        ChildSessionInfo {
            session_id: "child-a".to_string(),
            parent_session_id: "parent-gone".to_string(),
            agent_id: "agent-a".to_string(),
            depth: 1,
            mode: SpawnMode::Run,
            status: ChildSessionStatus::Active,
            timeout_secs: None,
            timeout_warning_secs: None,
            timeout_notify_interval_ratio: None,
            created_at: Instant::now(),
        },
    );
    mgr.children.write().await.register_child(
        "child-a",
        ChildSessionInfo {
            session_id: "grandchild".to_string(),
            parent_session_id: "child-a".to_string(),
            agent_id: "agent-b".to_string(),
            depth: 2,
            mode: SpawnMode::Run,
            status: ChildSessionStatus::Active,
            timeout_secs: None,
            timeout_warning_secs: None,
            timeout_notify_interval_ratio: None,
            created_at: Instant::now(),
        },
    );

    sweep_spawn_tree_reclaim(&mgr).await;

    assert!(
        mgr.children.read().await.find_child("child-a").is_none(),
        "child-a should be reclaimed"
    );
    assert!(
        mgr.children.read().await.find_child("grandchild").is_none(),
        "grandchild should be reclaimed"
    );
}
