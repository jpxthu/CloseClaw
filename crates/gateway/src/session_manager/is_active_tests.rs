//! Unit tests for `SessionManager::is_active()`.
//!
//! Covers:
//! - Session not found → false
//! - Session Idle → false
//! - Session Busy (llm_active) → true
//! - Session Waiting → true
//!
//! Tool-state tests are omitted because `register_tool_call` and
//! `update_tool_state` are `pub(crate)` in `closeclaw-session` and
//! cannot be called from gateway tests. Tool-state coverage is
//! provided by the session crate's own `exec_status` unit tests.

use super::test_helpers::setup_parent_with_conv;
use closeclaw_common::LlmState;

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
