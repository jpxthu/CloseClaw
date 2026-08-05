//! Unit tests for `compute_overall_timeout` (Step 1.6).
//!
//! Validates the overall yield timeout computation logic:
//! - Normal path: multiple children with different timeouts
//! - Edge cases: all None, single child, no children
//! - Fallback to DEFAULT_YIELD_TIMEOUT_SECS

use super::sessions_yield::compute_overall_timeout;
use crate::spawn::{ChildSessionInfo, ChildSessionStatus, SpawnMode};

/// Helper: build a minimal `ChildSessionInfo` with the given timeout.
fn make_child(timeout_secs: Option<u64>) -> ChildSessionInfo {
    ChildSessionInfo {
        session_id: "child-1".into(),
        parent_session_id: "parent-1".into(),
        agent_id: "agent-1".into(),
        depth: 1,
        mode: SpawnMode::Run,
        status: ChildSessionStatus::Active,
        timeout_secs,
        created_at: std::time::Instant::now(),
    }
}

/// Helper: build a `ChildSessionInfo` with a specific session_id.
fn make_child_with_id(id: &str, timeout_secs: Option<u64>) -> ChildSessionInfo {
    ChildSessionInfo {
        session_id: id.into(),
        parent_session_id: "parent-1".into(),
        agent_id: "agent-1".into(),
        depth: 1,
        mode: SpawnMode::Run,
        status: ChildSessionStatus::Active,
        timeout_secs,
        created_at: std::time::Instant::now(),
    }
}

// ── 1. Normal path: multiple children with different timeouts ───────────

/// Multiple children with different timeouts: overall = max + 60s.
#[test]
fn test_compute_overall_multiple_children_different_timeouts() {
    let children = vec![
        make_child_with_id("c1", Some(300)),
        make_child_with_id("c2", Some(600)),
        make_child_with_id("c3", Some(120)),
    ];
    // max = 600, + 60 = 660
    assert_eq!(compute_overall_timeout(&children), 660);
}

/// Three children where max is the first element.
#[test]
fn test_compute_overall_max_is_first() {
    let children = vec![
        make_child_with_id("c1", Some(900)),
        make_child_with_id("c2", Some(100)),
        make_child_with_id("c3", Some(50)),
    ];
    assert_eq!(compute_overall_timeout(&children), 960);
}

/// Three children where max is the last element.
#[test]
fn test_compute_overall_max_is_last() {
    let children = vec![
        make_child_with_id("c1", Some(50)),
        make_child_with_id("c2", Some(100)),
        make_child_with_id("c3", Some(900)),
    ];
    assert_eq!(compute_overall_timeout(&children), 960);
}

/// Multiple children with some None and some Some timeouts.
#[test]
fn test_compute_overall_mixed_timeouts() {
    let children = vec![
        make_child_with_id("c1", None),
        make_child_with_id("c2", Some(200)),
        make_child_with_id("c3", None),
        make_child_with_id("c4", Some(400)),
    ];
    // max of Some values = 400, + 60 = 460
    assert_eq!(compute_overall_timeout(&children), 460);
}

// ── 2. Edge case: all timeouts are None → use default ──────────────────

/// All children have None timeout → falls back to DEFAULT + 60.
#[test]
fn test_compute_overall_all_none_uses_default() {
    let children = vec![
        make_child_with_id("c1", None),
        make_child_with_id("c2", None),
    ];
    // DEFAULT_YIELD_TIMEOUT_SECS = 600, + 60 = 660
    assert_eq!(compute_overall_timeout(&children), 660);
}

/// Single child with None timeout → uses default.
#[test]
fn test_compute_overall_single_child_none() {
    let children = vec![make_child(None)];
    assert_eq!(compute_overall_timeout(&children), 660);
}

/// No children at all → uses default.
#[test]
fn test_compute_overall_no_children() {
    let children: Vec<ChildSessionInfo> = vec![];
    assert_eq!(compute_overall_timeout(&children), 660);
}

/// Single child with explicit timeout.
#[test]
fn test_compute_overall_single_child_with_timeout() {
    let children = vec![make_child(Some(300))];
    assert_eq!(compute_overall_timeout(&children), 360);
}

// ── 3. All children have the same timeout ──────────────────────────────

/// Multiple children all with the same timeout.
#[test]
fn test_compute_overall_all_same_timeout() {
    let children = vec![
        make_child_with_id("c1", Some(300)),
        make_child_with_id("c2", Some(300)),
        make_child_with_id("c3", Some(300)),
    ];
    assert_eq!(compute_overall_timeout(&children), 360);
}

// ── 4. Timeout value of 0 ─────────────────────────────────────────────

/// Child with timeout=0: max(0, ...) = 0, + 60 = 60.
#[test]
fn test_compute_overall_with_zero_timeout() {
    let children = vec![make_child(Some(0))];
    assert_eq!(compute_overall_timeout(&children), 60);
}

/// Mix of 0 and None: max of Some(0) = 0, + 60 = 60.
#[test]
fn test_compute_overall_zero_with_none() {
    let children = vec![make_child(None), make_child(Some(0))];
    assert_eq!(compute_overall_timeout(&children), 60);
}
