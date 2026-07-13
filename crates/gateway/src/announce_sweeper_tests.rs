//! Tests for `AnnounceSweeper` (Step 1.4).
//!
//! Covers:
//! - Normal path: idle child → announce pushed
//! - Skip path: running child → no announce
//! - Skip path: child not in children table → no announce
//! - Boundary: no children → run_once returns without action

use crate::announce_sweeper::AnnounceSweeper;
use crate::session_manager::test_helpers::{
    append_assistant_to_child, setup_parent_with_conv, test_resolved_config,
};
use crate::session_manager::tests::{clear_global_prompt_state, make_test_mgr};
use crate::session_manager::SpawnMode;
use closeclaw_common::LlmState;
use closeclaw_llm::types::ContentBlock;
use serial_test::serial;
use std::sync::Arc;
use tempfile::TempDir;

// ── 1. Normal path: idle child → try_push_announce called ────────────────

/// Child session is idle (three-dimensional status Idle) and still in
/// the children table — `run_once` should push an announce to the
/// parent's queue.
#[tokio::test]
#[serial]
async fn test_run_once_idle_child_pushes_announce() {
    clear_global_prompt_state();

    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-idle").await;

    let config = test_resolved_config("worker-idle", None);
    let child_id = mgr
        .create_child_session(
            &config,
            &parent_id,
            1,
            "do work",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None,
        )
        .await
        .expect("create_child_session should succeed");

    // Append an assistant message so extract_last_assistant_text succeeds.
    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![ContentBlock::Text("task complete".to_string())],
    )
    .await;

    let sm = Arc::new(mgr);
    let sweeper = AnnounceSweeper::new(Arc::clone(&sm));
    sweeper.run_once().await;

    let drained: Vec<_> = sm.drain_announces(&parent_id).await;
    assert_eq!(drained.len(), 1, "expected 1 announce event for idle child");
    assert_eq!(drained[0].child_session_id, child_id);
    assert_eq!(drained[0].result_text, "task complete");
}

// ── 2. Skip path: running child → no announce ────────────────────────────

/// Child session is still running (LLM state = Requesting) — `run_once`
/// should NOT push an announce.
#[tokio::test]
#[serial]
async fn test_run_once_running_child_skips() {
    clear_global_prompt_state();

    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-running").await;

    let config = test_resolved_config("worker-running", None);
    let child_id = mgr
        .create_child_session(
            &config,
            &parent_id,
            1,
            "do work",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None,
        )
        .await
        .expect("create_child_session should succeed");

    // Set child's LLM state to Requesting so exec_status returns Busy.
    {
        let cs = mgr
            .get_conversation_session(&child_id)
            .await
            .expect("child ConversationSession should exist");
        let cs = cs.write().await;
        cs.set_llm_state(LlmState::Requesting);
    }

    let sm = Arc::new(mgr);
    let sweeper = AnnounceSweeper::new(Arc::clone(&sm));
    sweeper.run_once().await;

    let drained: Vec<_> = sm.drain_announces(&parent_id).await;
    assert!(
        drained.is_empty(),
        "no announce should be pushed for a running child"
    );
}

// ── 3. Skip path: child not in children table → no announce ──────────────

/// Child has been removed from the children table (announce already
/// delivered) — `run_once` should skip it without error.
#[tokio::test]
#[serial]
async fn test_run_once_child_not_in_table_skips() {
    clear_global_prompt_state();

    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-removed").await;

    let config = test_resolved_config("worker-removed", None);
    let child_id = mgr
        .create_child_session(
            &config,
            &parent_id,
            1,
            "do work",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None,
        )
        .await
        .expect("create_child_session should succeed");

    // Append assistant message so the session is in a complete state.
    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![ContentBlock::Text("done".to_string())],
    )
    .await;

    // Remove child from the spawn tree (simulates announce already delivered).
    {
        let mut tree = mgr.children.write().await;
        tree.remove_child(&parent_id, &child_id);
    }

    let sm = Arc::new(mgr);
    let sweeper = AnnounceSweeper::new(Arc::clone(&sm));
    sweeper.run_once().await;

    let drained: Vec<_> = sm.drain_announces(&parent_id).await;
    assert!(
        drained.is_empty(),
        "no announce should be pushed for a child not in the table"
    );
}

// ── 4. Boundary: no children → run_once returns without action ───────────

/// No children registered in the spawn tree — `run_once` should return
/// without error or side effects.
#[tokio::test]
#[serial]
async fn test_run_once_no_children_returns_early() {
    clear_global_prompt_state();

    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-empty").await;

    let sm = Arc::new(mgr);
    let sweeper = AnnounceSweeper::new(Arc::clone(&sm));
    sweeper.run_once().await;

    let drained: Vec<_> = sm.drain_announces(&parent_id).await;
    assert!(
        drained.is_empty(),
        "no announce should be pushed when there are no children"
    );
}
