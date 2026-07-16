//! Tests for Step 1.5 — forceful termination notification + dedup (Gap 3).
//!
//! Validates:
//! - `notify_child_forced_termination` pushes Terminated announce
//! - `notify_child_error` sets child state to Errored
//! - Dedup protection: terminal states prevent duplicate notifications
//! - Child state transitions are correct after notifications

use super::test_helpers::{register_child_only, setup_parent_with_conv};
use super::tests::{clear_global_prompt_state, make_test_mgr};
use super::SessionManager;
use crate::session_manager::spawn::SpawnMode;
use crate::Session;
use chrono::Utc;
use closeclaw_common::{ChildCompletionStatus, ChildSessionState};
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_tasks::NotificationPriority;
use serial_test::serial;
use std::sync::Arc;

// ── helper ──────────────────────────────────────────────────────────────

async fn register_child_with_session(
    mgr: &SessionManager,
    parent_id: &str,
    child_id: &str,
    agent_id: &str,
    mode: SpawnMode,
) {
    register_child_only(mgr, parent_id, child_id, agent_id, mode).await;

    // Also register in parent's child_states so update_child_state works.
    {
        let parent_cs = mgr.get_conversation_session(parent_id).await.unwrap();
        let guard = parent_cs.read().await;
        guard.child_states.write().expect("lock").insert(
            child_id.to_string(),
            (closeclaw_common::ChildSessionState::Running, None),
        );
    }

    let cs = Arc::new(tokio::sync::RwLock::new(ConversationSession::new(
        child_id.to_string(),
        "test-model".to_string(),
        std::path::PathBuf::from("/tmp"),
    )));
    mgr.conversation_sessions
        .write()
        .await
        .insert(child_id.to_string(), cs);
    mgr.sessions.write().await.insert(
        child_id.to_string(),
        Session {
            id: child_id.to_string(),
            agent_id: agent_id.to_string(),
            channel: "feishu".to_string(),
            created_at: Utc::now().timestamp(),
            depth: 1,
        },
    );
}

// ── 1. Forceful kill pushes Terminated notification ─────────────────────

/// `notify_child_forced_termination` must push an AnnounceEvent with
/// `status = Terminated` to the parent session's queue.
#[tokio::test]
#[serial]
async fn test_forceful_kill_pushes_terminated_notification() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-ft-1").await;
    register_child_with_session(
        &mgr,
        &parent_id,
        "child-ft-1",
        "worker-ft-1",
        SpawnMode::Run,
    )
    .await;

    mgr.notify_child_forced_termination("child-ft-1").await;

    let drained = mgr.drain_announces(&parent_id).await;
    assert_eq!(drained.len(), 1, "expected 1 terminate notification");
    assert_eq!(
        drained[0].status,
        ChildCompletionStatus::Terminated,
        "notification status must be Terminated"
    );
    assert_eq!(drained[0].child_agent_id, "worker-ft-1");
}

// ── 2. Child state set to Terminated ────────────────────────────────────

/// After `notify_child_forced_termination`, the parent's `child_states`
/// must contain `ChildSessionState::Terminated` for the child.
#[tokio::test]
#[serial]
async fn test_forceful_kill_sets_child_state_terminated() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-ft-2").await;
    register_child_with_session(
        &mgr,
        &parent_id,
        "child-ft-2",
        "worker-ft-2",
        SpawnMode::Run,
    )
    .await;

    mgr.notify_child_forced_termination("child-ft-2").await;

    let parent_cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let guard = parent_cs.read().await;
    let states = guard.child_states.read().expect("lock");
    let state = states
        .get("child-ft-2")
        .map(|(s, _)| *s)
        .expect("child state should exist");
    assert_eq!(
        state,
        ChildSessionState::Terminated,
        "child state must be Terminated"
    );
}

// ── 3. Dedup: already-terminated child is not re-notified ───────────────

/// If the child's state is already `Terminated`, calling
/// `notify_child_forced_termination` again must not push a duplicate.
#[tokio::test]
#[serial]
async fn test_dedup_already_terminated() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-ded-1").await;
    register_child_with_session(
        &mgr,
        &parent_id,
        "child-ded-1",
        "worker-ded-1",
        SpawnMode::Run,
    )
    .await;

    // First call: pushes notification, sets state to Terminated.
    mgr.notify_child_forced_termination("child-ded-1").await;
    let drained1 = mgr.drain_announces(&parent_id).await;
    assert_eq!(drained1.len(), 1, "first call should push 1 notification");

    // Second call: state is already Terminated — should NOT push.
    mgr.notify_child_forced_termination("child-ded-1").await;
    let drained2 = mgr.drain_announces(&parent_id).await;
    assert!(
        drained2.is_empty(),
        "second call should NOT push duplicate, got: {:?}",
        drained2
    );
}

// ── 4. Dedup: already-errored child is not re-notified ──────────────────

/// If the child's state is already `Errored`,
/// `notify_child_forced_termination` must skip.
#[tokio::test]
#[serial]
async fn test_dedup_already_errored() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-ded-2").await;
    register_child_with_session(
        &mgr,
        &parent_id,
        "child-ded-2",
        "worker-ded-2",
        SpawnMode::Run,
    )
    .await;

    // Set state to Errored manually.
    {
        let parent_cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        let guard = parent_cs.read().await;
        guard.update_child_state("child-ded-2", ChildSessionState::Errored);
    }

    mgr.notify_child_forced_termination("child-ded-2").await;
    let drained = mgr.drain_announces(&parent_id).await;
    assert!(
        drained.is_empty(),
        "should not notify when child is already Errored, got: {:?}",
        drained
    );
}

// ── 5. Dedup: already-completed child is not re-notified ────────────────

/// If the child's state is already `Completed`,
/// `notify_child_forced_termination` must skip.
#[tokio::test]
#[serial]
async fn test_dedup_already_completed() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-ded-3").await;
    register_child_with_session(
        &mgr,
        &parent_id,
        "child-ded-3",
        "worker-ded-3",
        SpawnMode::Run,
    )
    .await;

    // Set state to Completed manually.
    {
        let parent_cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        let guard = parent_cs.read().await;
        guard.update_child_state("child-ded-3", ChildSessionState::Completed);
    }

    mgr.notify_child_forced_termination("child-ded-3").await;
    let drained = mgr.drain_announces(&parent_id).await;
    assert!(
        drained.is_empty(),
        "should not notify when child is already Completed, got: {:?}",
        drained
    );
}

// ── 6. Session-mode child is not notified ───────────────────────────────

/// `notify_child_forced_termination` must skip session-mode children
/// (only run-mode produces notifications).
#[tokio::test]
#[serial]
async fn test_forceful_kill_session_mode_no_notification() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-ft-sm").await;
    register_child_with_session(
        &mgr,
        &parent_id,
        "child-ft-sm",
        "worker-ft-sm",
        SpawnMode::Session,
    )
    .await;

    mgr.notify_child_forced_termination("child-ft-sm").await;

    let drained = mgr.drain_announces(&parent_id).await;
    assert!(
        drained.is_empty(),
        "session-mode child should not trigger terminate notification, got: {:?}",
        drained
    );
}

// ── 7. Non-child session is not notified ────────────────────────────────

/// `notify_child_forced_termination` called on a non-child session id
/// must be a no-op.
#[tokio::test]
#[serial]
async fn test_forceful_kill_non_child_noop() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-ft-nc").await;

    mgr.notify_child_forced_termination("not-a-child").await;

    let drained = mgr.drain_announces(&parent_id).await;
    assert!(drained.is_empty());
}

// ── 8. notify_child_error sets child state to Errored ───────────────────

/// `notify_child_error` must update the child's state to `Errored`
/// in the parent's `child_states` map.
#[tokio::test]
#[serial]
async fn test_notify_child_error_sets_state() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-ne-1").await;
    register_child_with_session(
        &mgr,
        &parent_id,
        "child-ne-1",
        "worker-ne-1",
        SpawnMode::Run,
    )
    .await;

    mgr.notify_child_error("child-ne-1").await;

    let parent_cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let guard = parent_cs.read().await;
    let states = guard.child_states.read().expect("lock");
    let state = states
        .get("child-ne-1")
        .map(|(s, _)| *s)
        .expect("child state should exist");
    assert_eq!(
        state,
        ChildSessionState::Errored,
        "child state must be Errored"
    );
}

// ── 9. notify_child_error dedup: terminal state ─────────────────────────

/// `notify_child_error` must not update state if child is already
/// in a terminal state (Completed, Errored, or Terminated).
#[tokio::test]
#[serial]
async fn test_notify_child_error_dedup_terminal() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-ne-d").await;
    register_child_with_session(
        &mgr,
        &parent_id,
        "child-ne-d",
        "worker-ne-d",
        SpawnMode::Run,
    )
    .await;

    // Set to Completed first.
    {
        let parent_cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        let guard = parent_cs.read().await;
        guard.update_child_state("child-ne-d", ChildSessionState::Completed);
    }

    mgr.notify_child_error("child-ne-d").await;

    let parent_cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let guard = parent_cs.read().await;
    let states = guard.child_states.read().expect("lock");
    let state = states.get("child-ne-d").map(|(s, _)| *s).unwrap();
    assert_eq!(
        state,
        ChildSessionState::Completed,
        "state should remain Completed, not overwritten"
    );
}

// ── 10. Terminated notification includes correct text ───────────────────

/// The AnnounceEvent pushed by `notify_child_forced_termination` must
/// contain "任务被终止" in its result_text.
#[tokio::test]
#[serial]
async fn test_terminated_notification_text() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-ft-t").await;
    register_child_with_session(
        &mgr,
        &parent_id,
        "child-ft-t",
        "worker-ft-t",
        SpawnMode::Run,
    )
    .await;

    mgr.notify_child_forced_termination("child-ft-t").await;

    let drained = mgr.drain_announces(&parent_id).await;
    assert_eq!(drained.len(), 1);
    assert_eq!(
        drained[0].result_text, "任务被终止",
        "result_text must be '任务被终止'"
    );
    assert_eq!(drained[0].priority, NotificationPriority::Next);
}
