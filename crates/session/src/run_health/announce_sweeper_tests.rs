//! Tests for `AnnounceSweeper` using a mock `AnnounceSweepTarget`.
//!
//! Covers:
//! - Normal path: idle child → announce pushed
//! - Skip path: running child → no announce
//! - Skip path: child removed from table → no announce
//! - Boundary: no children → run_once returns without action

use super::announce_sweeper::{AnnounceSweepTarget, AnnounceSweeper};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Mock target for testing `AnnounceSweeper` without a real
/// `SessionManager`.
struct MockTarget {
    children: RwLock<Vec<(String, String)>>,
    idle_sessions: RwLock<Vec<String>>,
    removed_children: RwLock<Vec<String>>,
    pushed_announces: RwLock<Vec<String>>,
}

impl MockTarget {
    fn new() -> Self {
        Self {
            children: RwLock::new(Vec::new()),
            idle_sessions: RwLock::new(Vec::new()),
            removed_children: RwLock::new(Vec::new()),
            pushed_announces: RwLock::new(Vec::new()),
        }
    }

    async fn add_child(&self, child_id: &str, parent_id: &str) {
        self.children
            .write()
            .await
            .push((child_id.to_string(), parent_id.to_string()));
    }

    async fn set_idle(&self, session_id: &str) {
        self.idle_sessions
            .write()
            .await
            .push(session_id.to_string());
    }

    async fn set_removed(&self, child_id: &str) {
        self.removed_children
            .write()
            .await
            .push(child_id.to_string());
    }

    async fn pushed_announces(&self) -> Vec<String> {
        self.pushed_announces.read().await.clone()
    }
}

#[async_trait]
impl AnnounceSweepTarget for MockTarget {
    async fn get_run_mode_children(&self) -> Vec<(String, String)> {
        self.children.read().await.clone()
    }

    async fn is_child_removed(&self, child_id: &str) -> bool {
        self.removed_children
            .read()
            .await
            .contains(&child_id.to_string())
    }

    async fn is_session_idle(&self, session_id: &str) -> bool {
        self.idle_sessions
            .read()
            .await
            .contains(&session_id.to_string())
    }

    async fn try_push_announce(&self, session_id: &str) {
        self.pushed_announces
            .write()
            .await
            .push(session_id.to_string());
    }
}

// ── 1. Normal path: idle child → try_push_announce called ────────────────

/// Child session is idle and still in the children table — `run_once`
/// should push an announce to the parent.
#[tokio::test]
async fn test_run_once_idle_child_pushes_announce() {
    let target = Arc::new(MockTarget::new());
    target.add_child("child-1", "parent-1").await;
    target.set_idle("child-1").await;

    let sweeper = AnnounceSweeper::new(target.clone());
    sweeper.run_once().await;

    let pushed = target.pushed_announces().await;
    assert_eq!(pushed.len(), 1, "expected 1 announce for idle child");
    assert_eq!(pushed[0], "child-1");
}

// ── 2. Skip path: running child → no announce ────────────────────────────

/// Child session is still running — `run_once` should NOT push.
#[tokio::test]
async fn test_run_once_running_child_skips() {
    let target = Arc::new(MockTarget::new());
    target.add_child("child-2", "parent-2").await;
    // child-2 is NOT idle

    let sweeper = AnnounceSweeper::new(target.clone());
    sweeper.run_once().await;

    let pushed = target.pushed_announces().await;
    assert!(pushed.is_empty(), "no announce for running child");
}

// ── 3. Skip path: child removed from table → no announce ────────────────

/// Child has been removed from the children table — `run_once` skips it.
#[tokio::test]
async fn test_run_once_child_not_in_table_skips() {
    let target = Arc::new(MockTarget::new());
    target.add_child("child-3", "parent-3").await;
    target.set_idle("child-3").await;
    target.set_removed("child-3").await;

    let sweeper = AnnounceSweeper::new(target.clone());
    sweeper.run_once().await;

    let pushed = target.pushed_announces().await;
    assert!(pushed.is_empty(), "no announce for removed child");
}

// ── 4. Boundary: no children → run_once returns without action ───────────

/// No children registered — `run_once` should return early.
#[tokio::test]
async fn test_run_once_no_children_returns_early() {
    let target = Arc::new(MockTarget::new());
    let sweeper = AnnounceSweeper::new(target.clone());
    sweeper.run_once().await;

    let pushed = target.pushed_announces().await;
    assert!(pushed.is_empty(), "no announce when there are no children");
}

// ── 5. Multiple children: mixed states ──────────────────────────────────

/// Mix of idle, running, and removed children — only idle non-removed
/// children should receive announce.
#[tokio::test]
async fn test_run_once_mixed_children() {
    let target = Arc::new(MockTarget::new());
    target.add_child("idle-child", "parent").await;
    target.add_child("running-child", "parent").await;
    target.add_child("removed-child", "parent").await;
    target.set_idle("idle-child").await;
    target.set_idle("removed-child").await;
    target.set_removed("removed-child").await;

    let sweeper = AnnounceSweeper::new(target.clone());
    sweeper.run_once().await;

    let pushed = target.pushed_announces().await;
    assert_eq!(
        pushed.len(),
        1,
        "only idle non-removed child should be pushed"
    );
    assert_eq!(pushed[0], "idle-child");
}
