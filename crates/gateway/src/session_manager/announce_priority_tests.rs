//! Tests for announce priority parameter passing (Gap 1).
//!
//! Validates that:
//! - `try_push_announce` uses the caller-specified priority on the event
//! - `notify_child_forced_termination` produces `Next` priority events
//! - Normal completion path still uses `Next` priority

use super::spawn::SpawnMode;
use super::test_helpers::{
    append_assistant_to_child, setup_parent_with_conv, test_resolved_config,
};
use super::tests::{clear_global_prompt_state, make_test_mgr};
use closeclaw_llm::types::ContentBlock;
use closeclaw_tasks::NotificationPriority;
use serial_test::serial;
use tempfile::TempDir;

// ── try_push_announce uses caller-specified priority ────────────────────────

/// When `try_push_announce` is called with `NotificationPriority::Later`,
/// the resulting announce event must carry `Later` priority.
#[tokio::test]
#[serial]
async fn test_try_push_announce_passes_later_priority() {
    clear_global_prompt_state();

    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-prio-later").await;

    let child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-prio-later", None),
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
            None,
            None,
        )
        .await
        .expect("create_child_session should succeed");

    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![ContentBlock::Text("done".to_string())],
    )
    .await;

    mgr.try_push_announce(&child_id, NotificationPriority::Later)
        .await;

    let drained = mgr.drain_announces(&parent_id).await;
    assert_eq!(drained.len(), 1, "expected 1 announce event");
    assert_eq!(
        drained[0].priority,
        NotificationPriority::Later,
        "event priority should match caller-specified Later"
    );
}

/// When `try_push_announce` is called with `NotificationPriority::Now`,
/// the resulting announce event must carry `Now` priority.
#[tokio::test]
#[serial]
async fn test_try_push_announce_passes_now_priority() {
    clear_global_prompt_state();

    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-prio-now").await;

    let child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-prio-now", None),
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
            None,
            None,
        )
        .await
        .expect("create_child_session should succeed");

    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![ContentBlock::Text("done".to_string())],
    )
    .await;

    mgr.try_push_announce(&child_id, NotificationPriority::Now)
        .await;

    let drained = mgr.drain_announces(&parent_id).await;
    assert_eq!(drained.len(), 1, "expected 1 announce event");
    assert_eq!(
        drained[0].priority,
        NotificationPriority::Now,
        "event priority should match caller-specified Now"
    );
}

// ── Normal completion path uses Next priority ───────────────────────────────

/// When `try_push_announce` is called with `NotificationPriority::Next`
/// (the standard path for normal child completion), the event must carry
/// `Next` priority.
#[tokio::test]
#[serial]
async fn test_try_push_announce_normal_completion_uses_next() {
    clear_global_prompt_state();

    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-prio-next").await;

    let child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-prio-next", None),
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
            None,
            None,
        )
        .await
        .expect("create_child_session should succeed");

    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![ContentBlock::Text("result text".to_string())],
    )
    .await;

    mgr.try_push_announce(&child_id, NotificationPriority::Next)
        .await;

    let drained = mgr.drain_announces(&parent_id).await;
    assert_eq!(drained.len(), 1, "expected 1 announce event");
    assert_eq!(
        drained[0].priority,
        NotificationPriority::Next,
        "normal completion should use Next priority"
    );
}

// ── notify_child_forced_termination produces Next priority ──────────────────

/// When a child is force-terminated, `notify_child_forced_termination`
/// must produce an `AnnounceEvent` with `NotificationPriority::Next`.
#[tokio::test]
#[serial]
async fn test_forced_termination_produces_next_priority() {
    clear_global_prompt_state();

    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-term-now").await;

    let child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-term-now", None),
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
            None,
            None,
        )
        .await
        .expect("create_child_session should succeed");

    // Append assistant text so extract_last_assistant_text has content.
    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![ContentBlock::Text("was terminated".to_string())],
    )
    .await;

    mgr.notify_child_forced_termination(&child_id).await;

    let drained = mgr.drain_announces(&parent_id).await;
    assert_eq!(
        drained.len(),
        1,
        "forced termination should produce exactly 1 event"
    );
    assert_eq!(
        drained[0].priority,
        NotificationPriority::Next,
        "forced termination must use Next priority"
    );
    assert_eq!(
        drained[0].status,
        closeclaw_common::ChildCompletionStatus::Terminated,
        "forced termination status must be Terminated"
    );
}
