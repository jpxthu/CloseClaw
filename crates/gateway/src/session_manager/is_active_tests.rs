//! Unit tests for `SessionManager::is_active()`.
//!
//! Covers:
//! - Session not found → false
//! - Session Idle → false
//! - Session Busy (llm_active) → true
//! - Session Waiting → true
//! - Session with running child (child_active) → true
//! - Session Idle + running child → true
//! - Session Idle + no running child → false
//!
//! Tool-state tests are omitted because `register_tool_call` and
//! `update_tool_state` are `pub(crate)` in `closeclaw-session` and
//! cannot be called from gateway tests. Tool-state coverage is
//! provided by the session crate's own `exec_status` unit tests.

use super::spawn::SpawnMode;
use super::test_helpers::{register_child_only, setup_parent_with_conv};
use closeclaw_common::{ChildSessionState, LlmState};

/// Helper: build a SessionManager with no persistence backend.
fn make_mgr() -> super::SessionManager {
    super::tests::make_test_mgr(None)
}

#[tokio::test]
async fn test_is_active_session_not_found() {
    let mgr = make_mgr();
    assert!(
        !mgr.is_active("nonexistent_session").await,
        "session not in memory should return false"
    );
}

#[tokio::test]
async fn test_is_active_session_idle() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "idle-session").await;

    // Default state is Idle — is_active should return false
    assert!(
        !mgr.is_active(&sid).await,
        "idle session should not be active"
    );
}

#[tokio::test]
async fn test_is_active_session_busy_llm() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "llm-busy-session").await;

    // Set LLM to Requesting → Busy
    {
        let cs = mgr.get_conversation_session(&sid).await.unwrap();
        let cs = cs.write().await;
        cs.set_llm_state(LlmState::Requesting);
    }

    assert!(
        mgr.is_active(&sid).await,
        "session with active LLM should be active"
    );
}

#[tokio::test]
async fn test_is_active_session_busy_receiving() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "llm-receiving-session").await;

    // Set LLM to Receiving → Busy
    {
        let cs = mgr.get_conversation_session(&sid).await.unwrap();
        let cs = cs.write().await;
        cs.set_llm_state(LlmState::Receiving);
    }

    assert!(
        mgr.is_active(&sid).await,
        "session with LLM in Receiving state should be active"
    );
}

#[tokio::test]
async fn test_is_active_session_waiting() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "waiting-session").await;

    // Trigger the waiting (yielding) state
    {
        let cs = mgr.get_conversation_session(&sid).await.unwrap();
        let cs = cs.write().await;
        cs.enter_waiting();
    }

    assert!(
        mgr.is_active(&sid).await,
        "session in Waiting state should be active"
    );
}

#[tokio::test]
async fn test_is_active_llm_returns_to_idle() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "transient-busy-session").await;

    // Set LLM to Requesting → Busy → is_active = true
    {
        let cs = mgr.get_conversation_session(&sid).await.unwrap();
        let cs = cs.write().await;
        cs.set_llm_state(LlmState::Requesting);
    }
    assert!(mgr.is_active(&sid).await);

    // Return LLM to Idle → is_active = false
    {
        let cs = mgr.get_conversation_session(&sid).await.unwrap();
        let cs = cs.write().await;
        cs.set_llm_state(LlmState::Idle);
    }
    assert!(
        !mgr.is_active(&sid).await,
        "session should not be active after LLM returns to idle"
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
async fn test_is_active_with_running_child_only() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "parent-child-active").await;

    // LLM is Idle (default), no tool work — only child_active is true.
    setup_child_running(&mgr, &sid, "child-1").await;

    assert!(
        mgr.is_active(&sid).await,
        "session with running child should be active even when exec_status is Idle"
    );
}

#[tokio::test]
async fn test_is_active_child_terminated_no_active_dims() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "parent-child-terminated").await;

    // Register child, then mark it Terminated.
    setup_child_running(&mgr, &sid, "child-2").await;
    set_child_terminated(&mgr, &sid, "child-2").await;

    assert!(
        !mgr.is_active(&sid).await,
        "session with only terminated child and idle exec_status should not be active"
    );
}

#[tokio::test]
async fn test_is_active_llm_busy_plus_running_child() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "parent-busy-child").await;

    // Both LLM busy AND child running — still active.
    {
        let cs = mgr.get_conversation_session(&sid).await.unwrap();
        let cs = cs.write().await;
        cs.set_llm_state(LlmState::Requesting);
    }
    setup_child_running(&mgr, &sid, "child-3").await;

    assert!(
        mgr.is_active(&sid).await,
        "session with LLM busy and running child should be active"
    );
}

#[tokio::test]
async fn test_is_active_llm_idle_child_terminates() {
    let mgr = make_mgr();
    let sid = setup_parent_with_conv(&mgr, "parent-child-lifecycle").await;

    // Start with running child → active.
    setup_child_running(&mgr, &sid, "child-4").await;
    assert!(mgr.is_active(&sid).await);

    // Child terminates → no active dimensions → not active.
    set_child_terminated(&mgr, &sid, "child-4").await;
    assert!(
        !mgr.is_active(&sid).await,
        "session should not be active after child terminates and LLM is idle"
    );
}
