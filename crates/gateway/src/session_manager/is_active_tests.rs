//! Unit tests for `SessionManager::activity_dimensions()`.
//!
//! Covers:
//! - Session not found → all false
//! - Session Idle → all false
//! - Session Busy (llm_active) → llm_active=true
//! - Session Waiting → may have all false (Waiting is not a four-dim dimension)
//! - Session with running child (child_active) → child_active=true
//! - Session Idle + running child → child_active=true
//! - Session Idle + no running child → all false
//!
//! Tool-state tests are omitted because `register_tool_call` and
//! `update_tool_state` are `pub(crate)` in `closeclaw-session` and
//! cannot be called from gateway tests. Tool-state coverage is
//! provided by the session crate's own `activity_dimensions` unit tests.

use super::spawn::SpawnMode;
use super::test_helpers::{register_child_only, setup_parent_with_conv};
use closeclaw_common::{ChildSessionState, LlmState};

/// Helper: build a SessionManager with no persistence backend.
fn make_mgr() -> super::SessionManager {
    super::tests::make_test_mgr(None)
}

#[tokio::test]
async fn test_activity_dimensions_session_not_found() {
    let mgr = make_mgr();
    let dims = mgr.activity_dimensions("nonexistent_session").await;
    assert!(
        !dims.any_active(),
        "session not in memory should return all-false dimensions"
    );
}

#[tokio::test]
async fn test_activity_dimensions_session_idle() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "idle-session").await;

    // Default state is Idle — activity_dimensions should return all false
    let dims = mgr.activity_dimensions(&sid).await;
    assert!(
        !dims.any_active(),
        "idle session should have no active dimensions"
    );
}

#[tokio::test]
async fn test_activity_dimensions_session_busy_llm() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "llm-busy-session").await;

    // Set LLM to Requesting → llm_active=true
    {
        let cs = mgr.get_conversation_session(&sid).await.unwrap();
        let cs = cs.write().await;
        cs.set_llm_state(LlmState::Requesting);
    }

    let dims = mgr.activity_dimensions(&sid).await;
    assert!(
        dims.llm_active,
        "llm_active should be true when LLM is Requesting"
    );
    assert!(
        dims.any_active(),
        "session with active LLM should have at least one active dimension"
    );
}

#[tokio::test]
async fn test_activity_dimensions_session_busy_receiving() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "llm-receiving-session").await;

    // Set LLM to Receiving → llm_active=true
    {
        let cs = mgr.get_conversation_session(&sid).await.unwrap();
        let cs = cs.write().await;
        cs.set_llm_state(LlmState::Receiving);
    }

    let dims = mgr.activity_dimensions(&sid).await;
    assert!(
        dims.llm_active,
        "llm_active should be true when LLM is Receiving"
    );
    assert!(
        dims.any_active(),
        "session with LLM in Receiving state should have at least one active dimension"
    );
}

#[tokio::test]
async fn test_activity_dimensions_session_waiting() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "waiting-session").await;

    // Trigger the waiting (yielding) state
    {
        let cs = mgr.get_conversation_session(&sid).await.unwrap();
        let cs = cs.write().await;
        cs.enter_waiting();
    }

    let dims = mgr.activity_dimensions(&sid).await;
    // Waiting (yielding) is not a four-dim dimension; all dims may be false.
    // This is expected per the design doc — Waiting is not covered by the four dims.
    assert!(
        !dims.any_active(),
        "session in Waiting (yielding) state should have all-false dimensions per design doc"
    );
}

#[tokio::test]
async fn test_activity_dimensions_llm_returns_to_idle() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "transient-busy-session").await;

    // Set LLM to Requesting → llm_active=true
    {
        let cs = mgr.get_conversation_session(&sid).await.unwrap();
        let cs = cs.write().await;
        cs.set_llm_state(LlmState::Requesting);
    }
    assert!(mgr.activity_dimensions(&sid).await.any_active());

    // Return LLM to Idle → activity_dimensions should return all false
    {
        let cs = mgr.get_conversation_session(&sid).await.unwrap();
        let cs = cs.write().await;
        cs.set_llm_state(LlmState::Idle);
    }
    assert!(
        !mgr.activity_dimensions(&sid).await.any_active(),
        "session should have no active dimensions after LLM returns to idle"
    );
}

// ── child_active dimension tests ───────────────────────────────────────

/// Helper: register a child and insert it into parent's child_states
/// so `has_running_child()` returns true.
async fn setup_child_running(mgr: &super::SessionManager, parent_id: &str, child_id: &str) {
    register_child_only(mgr, parent_id, child_id, "child-agent", SpawnMode::Run).await;
    let cs = mgr.get_conversation_session(parent_id).await.unwrap();
    let guard = cs.read().await;
    guard
        .child_states
        .write()
        .expect("lock")
        .insert(child_id.to_string(), (ChildSessionState::Running, None));
}

/// Helper: set child state to Terminated so `has_running_child()`
/// returns false.
async fn set_child_terminated(mgr: &super::SessionManager, parent_id: &str, child_id: &str) {
    let cs = mgr.get_conversation_session(parent_id).await.unwrap();
    let guard = cs.read().await;
    guard
        .child_states
        .write()
        .expect("lock")
        .insert(child_id.to_string(), (ChildSessionState::Terminated, None));
}

#[tokio::test]
async fn test_activity_dimensions_with_running_child_only() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "parent-child-active").await;

    // LLM is Idle (default), no tool work — only child_active is true.
    setup_child_running(&mgr, &sid, "child-1").await;

    let dims = mgr.activity_dimensions(&sid).await;
    assert!(
        dims.child_active,
        "child_active should be true when a child session is running"
    );
    assert!(
        dims.any_active(),
        "session with running child should have at least one active dimension"
    );
}

#[tokio::test]
async fn test_activity_dimensions_child_terminated_no_active_dims() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "parent-child-terminated").await;

    // Register child, then mark it Terminated.
    setup_child_running(&mgr, &sid, "child-2").await;
    set_child_terminated(&mgr, &sid, "child-2").await;

    let dims = mgr.activity_dimensions(&sid).await;
    assert!(
        !dims.child_active,
        "child_active should be false when child is terminated"
    );
    assert!(
        !dims.any_active(),
        "session with only terminated child and idle exec_status should have no active dimensions"
    );
}

#[tokio::test]
async fn test_activity_dimensions_llm_busy_plus_running_child() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "parent-busy-child").await;

    // Both LLM busy AND child running
    {
        let cs = mgr.get_conversation_session(&sid).await.unwrap();
        let cs = cs.write().await;
        cs.set_llm_state(LlmState::Requesting);
    }
    setup_child_running(&mgr, &sid, "child-3").await;

    let dims = mgr.activity_dimensions(&sid).await;
    assert!(
        dims.llm_active,
        "llm_active should be true when LLM is Requesting"
    );
    assert!(
        dims.child_active,
        "child_active should be true when a child is running"
    );
    assert!(
        dims.any_active(),
        "session with LLM busy and running child should have at least one active dimension"
    );
}

#[tokio::test]
async fn test_activity_dimensions_llm_idle_child_terminates() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "parent-child-lifecycle").await;

    // Start with running child → child_active=true
    setup_child_running(&mgr, &sid, "child-4").await;
    assert!(mgr.activity_dimensions(&sid).await.any_active());

    // Child terminates → no active dimensions
    set_child_terminated(&mgr, &sid, "child-4").await;
    assert!(
        !mgr.activity_dimensions(&sid).await.any_active(),
        "session should have no active dimensions after child terminates and LLM is idle"
    );
}
