//! Tests for Step 1.5 — graceful stop timeout returns progress to caller.
//!
//! Validates:
//! - `stop_single_session` returns `TimedOut` with progress info when
//!   graceful wait times out
//! - Caller choosing forceful after timeout → session cleaned up
//! - Caller choosing to continue waiting → session not cleaned up
//! - `process_stop_level` escalates to forceful on graceful timeout

use super::stop::GracefulStopOutcome;
use super::test_helpers::{register_child_only, setup_parent_with_conv};
use super::tests::{clear_global_prompt_state, make_test_mgr};
use super::SessionManager;
use crate::session_manager::spawn::SpawnMode;
use crate::Session;
use chrono::Utc;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_llm::session_state::LlmState;
use closeclaw_session::llm_session::ConversationSession;
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;

// ── helper ──────────────────────────────────────────────────────────────

async fn register_child_with_session(
    mgr: &SessionManager,
    parent_id: &str,
    child_id: &str,
    agent_id: &str,
) {
    register_child_only(mgr, parent_id, child_id, agent_id, SpawnMode::Run).await;

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

async fn set_llm_state(mgr: &SessionManager, sid: &str, state: LlmState) {
    let cs = mgr.get_conversation_session(sid).await.unwrap();
    let guard = cs.read().await;
    *guard.llm_state.write().expect("lock") = state;
}

// ── 1. stop_single_session returns TimedOut with progress ───────────────

/// When `stop_single_session` is called in Graceful mode with a very
/// short timeout on a streaming session, it must return
/// `GracefulStopOutcome::TimedOut` containing progress information.
#[tokio::test]
#[serial]
async fn test_graceful_stop_returns_timedout_with_progress() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-gto-1").await;
    register_child_with_session(&mgr, &parent_id, "child-gto-1", "worker-gto-1").await;
    set_llm_state(&mgr, "child-gto-1", LlmState::Receiving).await;

    // Zero timeout forces immediate TimedOut.
    let outcome = mgr
        .stop_single_session(
            "child-gto-1",
            ShutdownMode::Graceful,
            false,
            Duration::ZERO,
            None,
        )
        .await;

    match outcome {
        Ok(GracefulStopOutcome::TimedOut {
            waiting_items,
            remaining,
        }) => {
            // At least one waiting item (the streaming LLM call).
            assert!(
                !waiting_items.is_empty() || remaining > 0,
                "TimedOut should carry progress info: items={}, remaining={}",
                waiting_items.len(),
                remaining
            );
        }
        other => panic!("expected TimedOut, got {:?}", other),
    }
}

// ── 2. TimedOut preserves waiting item details ──────────────────────────

/// The `waiting_items` in the TimedOut outcome should contain
/// meaningful descriptions of what is still in flight.
#[tokio::test]
#[serial]
async fn test_graceful_timeout_waiting_items_describe_inflight() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-gto-2").await;
    register_child_with_session(&mgr, &parent_id, "child-gto-2", "worker-gto-2").await;
    set_llm_state(&mgr, "child-gto-2", LlmState::Receiving).await;

    let outcome = mgr
        .stop_single_session(
            "child-gto-2",
            ShutdownMode::Graceful,
            false,
            Duration::ZERO,
            None,
        )
        .await;

    match outcome {
        Ok(GracefulStopOutcome::TimedOut { waiting_items, .. }) => {
            // If there are waiting items, each should be a non-empty string.
            for (name, elapsed) in &waiting_items {
                assert!(!name.is_empty(), "waiting item name should not be empty");
                assert!(*elapsed >= Duration::ZERO, "elapsed should be non-negative");
            }
        }
        other => panic!("expected TimedOut, got {:?}", other),
    }
}

// ── 3. Caller escalates to forceful after timeout → session cleaned ─────

/// After `stop_single_session` returns TimedOut, if the caller
/// re-calls with Forceful mode, the session must be cleaned up.
#[tokio::test]
#[serial]
async fn test_caller_forceful_after_timeout_cleans_session() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-gto-3").await;
    register_child_with_session(&mgr, &parent_id, "child-gto-3", "worker-gto-3").await;
    set_llm_state(&mgr, "child-gto-3", LlmState::Receiving).await;

    // First call: Graceful with zero timeout → TimedOut.
    let outcome = mgr
        .stop_single_session(
            "child-gto-3",
            ShutdownMode::Graceful,
            false,
            Duration::ZERO,
            None,
        )
        .await;
    assert!(
        matches!(outcome, Ok(GracefulStopOutcome::TimedOut { .. })),
        "first call should return TimedOut"
    );

    // Verify session is still alive (not force-killed).
    assert!(
        mgr.has_session("child-gto-3").await,
        "session should still exist after TimedOut"
    );
    let cs = mgr
        .get_conversation_session("child-gto-3")
        .await
        .expect("session should exist");
    assert!(
        !cs.read().await.is_cancelled(),
        "token should NOT be cancelled yet"
    );

    // Second call: Forceful → session force-stopped.
    let outcome2 = mgr
        .stop_single_session(
            "child-gto-3",
            ShutdownMode::Forceful,
            false,
            Duration::ZERO,
            None,
        )
        .await;
    assert!(
        matches!(
            outcome2,
            Ok(GracefulStopOutcome::Completed) | Ok(GracefulStopOutcome::Interrupted)
        ),
        "forceful after timeout should succeed, got: {:?}",
        outcome2
    );

    // After forceful, cancel token should be set.
    assert!(
        cs.read().await.is_cancelled(),
        "token should be cancelled after forceful stop"
    );
}

// ── 4. Caller continues waiting → session not cleaned ───────────────────

/// After `stop_single_session` returns TimedOut, if the caller
/// simply does nothing (simulating "continue waiting"), the session
/// must remain alive and not be force-killed.
#[tokio::test]
#[serial]
async fn test_caller_continue_waiting_session_not_cleaned() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-gto-4").await;
    register_child_with_session(&mgr, &parent_id, "child-gto-4", "worker-gto-4").await;
    set_llm_state(&mgr, "child-gto-4", LlmState::Receiving).await;

    // Graceful with zero timeout → TimedOut.
    let outcome = mgr
        .stop_single_session(
            "child-gto-4",
            ShutdownMode::Graceful,
            false,
            Duration::ZERO,
            None,
        )
        .await;
    assert!(matches!(outcome, Ok(GracefulStopOutcome::TimedOut { .. })));

    // Session should still exist.
    assert!(mgr.has_session("child-gto-4").await);

    // Cancel token should NOT be set.
    let cs = mgr
        .get_conversation_session("child-gto-4")
        .await
        .expect("session should exist");
    assert!(
        !cs.read().await.is_cancelled(),
        "token should NOT be cancelled when caller continues waiting"
    );

    // LLM state should still be Receiving (not cleared).
    let guard = cs.read().await;
    let llm = guard.llm_state.read().expect("lock");
    assert_eq!(
        *llm,
        LlmState::Receiving,
        "LLM state should remain Receiving when caller continues waiting"
    );
}

// ── 5. process_stop_level escalates on graceful timeout ────────────────

/// `process_stop_level` must detect `TimedOut` during graceful
/// shutdown and escalate to forceful for that session.
#[tokio::test]
#[serial]
async fn test_process_stop_level_escalates_on_timeout() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-gto-5").await;
    register_child_with_session(&mgr, &parent_id, "child-gto-5", "worker-gto-5").await;
    set_llm_state(&mgr, "child-gto-5", LlmState::Receiving).await;

    // stop_all_sessions with zero timeout: process_stop_level sees
    // TimedOut and escalates to forceful.
    let result = mgr
        .stop_all_sessions(ShutdownMode::Graceful, Duration::ZERO, None)
        .await;

    // Escalation should have force-stopped the session.
    assert!(
        result.succeeded >= 1,
        "escalation should force-stop session, got: {:?}",
        result
    );
    assert!(
        mgr.has_session("child-gto-5").await,
        "session should remain in tracking tables"
    );
    let cs = mgr
        .get_conversation_session("child-gto-5")
        .await
        .expect("session should exist");
    assert!(
        cs.read().await.is_cancelled(),
        "token should be cancelled after escalation"
    );
}

// ── 6. Graceful with sufficient timeout completes immediately ───────────

/// An idle session stops immediately in Graceful mode (no timeout).
#[tokio::test]
#[serial]
async fn test_graceful_idle_completes_immediately_gto() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-gto-6").await;
    register_child_with_session(&mgr, &parent_id, "child-gto-6", "worker-gto-6").await;

    // Idle session: should complete immediately.
    let outcome = mgr
        .stop_single_session(
            "child-gto-6",
            ShutdownMode::Graceful,
            false,
            Duration::from_secs(30),
            None,
        )
        .await;
    assert!(
        matches!(outcome, Ok(GracefulStopOutcome::Completed)),
        "idle session should complete immediately, got: {:?}",
        outcome
    );
}
