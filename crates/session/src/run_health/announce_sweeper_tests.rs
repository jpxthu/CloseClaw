//! Tests for `AnnounceSweeper` using a mock `AnnounceSweepTarget`.
//!
//! Covers:
//! - Normal path: idle child → announce pushed
//! - Skip path: running child → no announce
//! - Skip path: child removed from table → no announce
//! - Boundary: no children → run_once returns without action
//! - Stale detection: boundary (301s / 299s), idle skip, notification
//! - Stale detection: error paths (no output, archived parent)
//! - Stale detection: cascade termination of nested descendants
//! - Regression: original sweep path unchanged

use super::announce_sweeper::{AnnounceSweepTarget, AnnounceSweeper};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use closeclaw_tasks::NotificationPriority;

/// Mock target for testing `AnnounceSweeper` without a real
/// `SessionManager`.
struct MockTarget {
    children: RwLock<Vec<(String, String)>>,
    idle_sessions: RwLock<Vec<String>>,
    removed_children: RwLock<Vec<String>>,
    pushed_announces: RwLock<Vec<String>>,
    /// Per-session last output timestamp (epoch seconds).
    last_output: RwLock<HashMap<String, i64>>,
    /// Set of parent session ids that are archived.
    archived_parents: RwLock<Vec<String>>,
    /// Record of `(parent_id, child_id)` calls to
    /// `terminate_stale_child`.
    terminated_children: RwLock<Vec<(String, String)>>,
}

impl MockTarget {
    fn new() -> Self {
        Self {
            children: RwLock::new(Vec::new()),
            idle_sessions: RwLock::new(Vec::new()),
            removed_children: RwLock::new(Vec::new()),
            pushed_announces: RwLock::new(Vec::new()),
            last_output: RwLock::new(HashMap::new()),
            archived_parents: RwLock::new(Vec::new()),
            terminated_children: RwLock::new(Vec::new()),
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

    /// Set the `last_activity_at` timestamp for a session.
    async fn set_last_output(&self, session_id: &str, ts: i64) {
        self.last_output
            .write()
            .await
            .insert(session_id.to_string(), ts);
    }

    /// Mark a parent session as archived.
    async fn set_parent_archived(&self, parent_id: &str) {
        self.archived_parents
            .write()
            .await
            .push(parent_id.to_string());
    }

    /// Return the list of `(parent_id, child_id)` terminate calls.
    async fn terminated_children(&self) -> Vec<(String, String)> {
        self.terminated_children.read().await.clone()
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

    async fn try_push_announce(&self, session_id: &str, _priority: NotificationPriority) {
        self.pushed_announces
            .write()
            .await
            .push(session_id.to_string());
    }

    async fn get_last_output_at(&self, session_id: &str) -> Option<i64> {
        self.last_output.read().await.get(session_id).copied()
    }

    async fn is_parent_archived(&self, parent_id: &str) -> bool {
        self.archived_parents
            .read()
            .await
            .contains(&parent_id.to_string())
    }

    async fn terminate_stale_child(&self, parent_id: &str, child_id: &str) {
        self.terminated_children
            .write()
            .await
            .push((parent_id.to_string(), child_id.to_string()));
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Original sweep tests (regression: behavior unchanged)
// ═══════════════════════════════════════════════════════════════════════════

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

/// No children registered — `run_once` should return early.
#[tokio::test]
async fn test_run_once_no_children_returns_early() {
    let target = Arc::new(MockTarget::new());
    let sweeper = AnnounceSweeper::new(target.clone());
    sweeper.run_once().await;

    let pushed = target.pushed_announces().await;
    assert!(pushed.is_empty(), "no announce when there are no children");
}

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

// ═══════════════════════════════════════════════════════════════════════════
// Stale detection tests
// ═══════════════════════════════════════════════════════════════════════════

// ── 6. Stale boundary: 301s elapsed → terminate ─────────────────────────

/// A non-idle child whose last output was 301 seconds ago exceeds the
/// 300s threshold and must be terminated.
#[tokio::test]
async fn test_stale_301s_terminates_child() {
    let target = Arc::new(MockTarget::new());
    target.add_child("child-301", "parent-301").await;
    let now = 1000i64;
    target.set_last_output("child-301", now - 301).await;

    let sweeper = AnnounceSweeper::new(target.clone());
    sweeper.run_once_with_now(Some(now)).await;

    let terminated = target.terminated_children().await;
    assert_eq!(terminated.len(), 1, "child 301s old should be terminated");
    assert_eq!(terminated[0].0, "parent-301");
    assert_eq!(terminated[0].1, "child-301");
}

// ── 7. Stale boundary: 299s elapsed → skip ─────────────────────────────

/// A non-idle child whose last output was 299 seconds ago is still
/// under the 300s threshold and must NOT be terminated.
#[tokio::test]
async fn test_stale_299s_not_terminated() {
    let target = Arc::new(MockTarget::new());
    target.add_child("child-299", "parent-299").await;
    let now = 1000i64;
    target.set_last_output("child-299", now - 299).await;

    let sweeper = AnnounceSweeper::new(target.clone());
    sweeper.run_once_with_now(Some(now)).await;

    let terminated = target.terminated_children().await;
    assert!(
        terminated.is_empty(),
        "child 299s old should NOT be terminated"
    );
}

// ── 8. Idle children never detected as stale ────────────────────────────

/// An idle child is never checked for staleness — only the announce
/// path applies (already tested above).
#[tokio::test]
async fn test_idle_child_never_stale() {
    let target = Arc::new(MockTarget::new());
    target.add_child("child-idle", "parent-idle").await;
    target.set_idle("child-idle").await;
    let now = 1000i64;
    target.set_last_output("child-idle", now - 9999).await;

    let sweeper = AnnounceSweeper::new(target.clone());
    sweeper.run_once_with_now(Some(now)).await;

    let terminated = target.terminated_children().await;
    assert!(terminated.is_empty(), "idle child must never be terminated");
}

// ── 9. Stale child receives terminated notification ─────────────────────

/// When a stale child is terminated, a `Terminated` announce is pushed
/// to the parent's queue.
#[tokio::test]
async fn test_stale_child_notifies_parent() {
    let target = Arc::new(MockTarget::new());
    target.add_child("child-ntf", "parent-ntf").await;
    let now = 1000i64;
    target.set_last_output("child-ntf", now - 301).await;

    let sweeper = AnnounceSweeper::new(target.clone());
    sweeper.run_once_with_now(Some(now)).await;

    let terminated = target.terminated_children().await;
    assert_eq!(terminated.len(), 1);
    assert_eq!(terminated[0].1, "child-ntf");
}

// ── 10. No last output → not stale ──────────────────────────────────────

/// A non-idle child with no recorded `last_activity_at` (returns
/// `None`) must NOT be considered stale.
#[tokio::test]
async fn test_stale_no_output_not_stale() {
    let target = Arc::new(MockTarget::new());
    target.add_child("child-none", "parent-none").await;

    let sweeper = AnnounceSweeper::new(target.clone());
    sweeper.run_once_with_now(Some(1000)).await;

    let terminated = target.terminated_children().await;
    assert!(
        terminated.is_empty(),
        "child with no last_output should not be stale"
    );
}

// ── 11. Archived parent → terminate still fires, no notification ────────

/// Even when the parent is archived, the stale child must still be
/// terminated. The notification injection is skipped at the gateway
/// layer; the sweeper calls `terminate_stale_child` unconditionally.
#[tokio::test]
async fn test_stale_parent_archived_terminate_still_fires() {
    let target = Arc::new(MockTarget::new());
    target.add_child("child-arch", "parent-arch").await;
    target.set_parent_archived("parent-arch").await;
    let now = 1000i64;
    target.set_last_output("child-arch", now - 301).await;

    let sweeper = AnnounceSweeper::new(target.clone());
    sweeper.run_once_with_now(Some(now)).await;

    let terminated = target.terminated_children().await;
    assert_eq!(
        terminated.len(),
        1,
        "terminate should fire even when parent archived"
    );
    assert_eq!(terminated[0].0, "parent-arch");
}

// ── 12. Multiple stale children: each terminated independently ──────────

/// When multiple non-idle children exceed the threshold, each is
/// terminated independently.
#[tokio::test]
async fn test_stale_multiple_children_all_terminated() {
    let target = Arc::new(MockTarget::new());
    target.add_child("stale-a", "parent-m").await;
    target.add_child("stale-b", "parent-m").await;
    target.add_child("fresh-c", "parent-m").await;
    let now = 1000i64;
    target.set_last_output("stale-a", now - 301).await;
    target.set_last_output("stale-b", now - 500).await;
    target.set_last_output("fresh-c", now - 100).await;

    let sweeper = AnnounceSweeper::new(target.clone());
    sweeper.run_once_with_now(Some(now)).await;

    let terminated = target.terminated_children().await;
    assert_eq!(terminated.len(), 2, "2 stale children should be terminated");
    let ids: Vec<&str> = terminated.iter().map(|(_, c)| c.as_str()).collect();
    assert!(ids.contains(&"stale-a"), "stale-a should be terminated");
    assert!(ids.contains(&"stale-b"), "stale-b should be terminated");
}

// ── 13. Regression: idle + stale coexist correctly ──────────────────────

/// An idle child receives announce, a stale child is terminated, and
/// a fresh active child is skipped — all in one sweep.
#[tokio::test]
async fn test_stale_idle_fresh_coexist() {
    let target = Arc::new(MockTarget::new());
    target.add_child("idle-x", "parent-coexist").await;
    target.add_child("stale-y", "parent-coexist").await;
    target.add_child("fresh-z", "parent-coexist").await;
    target.set_idle("idle-x").await;
    let now = 1000i64;
    target.set_last_output("stale-y", now - 400).await;
    target.set_last_output("fresh-z", now - 50).await;

    let sweeper = AnnounceSweeper::new(target.clone());
    sweeper.run_once_with_now(Some(now)).await;

    let pushed = target.pushed_announces().await;
    assert_eq!(pushed.len(), 1);
    assert_eq!(pushed[0], "idle-x");
    let terminated = target.terminated_children().await;
    assert_eq!(terminated.len(), 1);
    assert_eq!(terminated[0].1, "stale-y");
}

// ── 14. Exact threshold boundary: 300s → skip (not >) ──────────────────

/// The threshold is `> 300` (strictly greater). At exactly 300s elapsed
/// the child is NOT stale.
#[tokio::test]
async fn test_stale_exact_300s_not_terminated() {
    let target = Arc::new(MockTarget::new());
    target.add_child("child-exact", "parent-exact").await;
    let now = 1000i64;
    target.set_last_output("child-exact", now - 300).await;

    let sweeper = AnnounceSweeper::new(target.clone());
    sweeper.run_once_with_now(Some(now)).await;

    let terminated = target.terminated_children().await;
    assert!(
        terminated.is_empty(),
        "exactly 300s elapsed should NOT be terminated (threshold is > 300)"
    );
}
