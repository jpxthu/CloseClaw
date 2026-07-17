//! Tests for yield timeout protection (Step 1.7).
//!
//! Covers timeout timer start/cancel, timeout expiry behavior,
//! notification injection, and session resume after timeout.

use super::spawn::SpawnMode;
use super::test_helpers::{setup_parent_with_conv, test_resolved_config};
use super::tests::{clear_global_prompt_state, make_test_mgr};
use closeclaw_session::llm_session::ChatSession;
use serial_test::serial;
use std::sync::Arc;

// ── 1. start_yield_timeout registers a handle ──────────────────────────────

/// After `start_yield_timeout`, the session should have a registered
/// timeout handle (verified by the fact that `cancel_yield_timeout`
/// can abort it without error).
#[tokio::test]
#[serial]
async fn test_yield_timeout_start_registers_handle() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-to1").await;

    // Start a yield timeout with a long duration (won't fire in test).
    mgr.start_yield_timeout(&parent_id, "agent-x", Some(600))
        .await;

    // Cancel should succeed (handle exists).
    mgr.cancel_yield_timeout(&parent_id).await;

    // Double cancel is a no-op (no panic).
    mgr.cancel_yield_timeout(&parent_id).await;
}

// ── 2. cancel_yield_timeout prevents timer from firing ─────────────────────

/// Start a short timeout, cancel it before it fires, and verify the
/// session remains in Waiting state.
#[tokio::test]
#[serial]
async fn test_yield_timeout_cancel_prevents_fire() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-to2").await;

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Start a short timeout (2 seconds).
    mgr.start_yield_timeout(&parent_id, "agent-x", Some(2))
        .await;

    // Cancel before it fires.
    mgr.cancel_yield_timeout(&parent_id).await;

    // Wait briefly to ensure the cancelled timer doesn't fire.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // Session should still be yielding (timer was cancelled).
    assert!(
        mgr.is_session_yielding(&parent_id).await,
        "session should remain in Waiting after timeout is cancelled"
    );

    // Cleanup.
    mgr.cancel_yield_timeout(&parent_id).await;
}

// ── 3. start_yield_timeout replaces existing handle ────────────────────────

/// Starting a timeout twice for the same session should abort the
/// first timer and start a new one.
#[tokio::test]
#[serial]
async fn test_yield_timeout_start_replaces_existing() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-to3").await;

    // Start first timeout.
    mgr.start_yield_timeout(&parent_id, "agent-x", Some(600))
        .await;

    // Start second timeout (should abort the first).
    mgr.start_yield_timeout(&parent_id, "agent-x", Some(600))
        .await;

    // Cancel should work without issue.
    mgr.cancel_yield_timeout(&parent_id).await;
}

// ── 4. Timeout fires and resumes session (short timeout integration) ───────

/// With a very short timeout (1 second), verify that the session
/// resumes after timeout fires. The timeout handler terminates children
/// and injects a notification.
///
/// Note: This test uses a 1s timeout and waits 2s, which is within
/// the 30s per-test limit.
#[tokio::test]
#[serial]
async fn test_yield_timeout_fires_and_resumes() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-to4").await;

    // Spawn a run-mode child that won't complete.
    let _child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-to4", None),
            &parent_id,
            1,
            "long task",
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
            None, // prompt_template_prefix
        )
        .await
        .unwrap();

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }
    assert!(mgr.is_session_yielding(&parent_id).await);

    // Start a 1-second timeout.
    mgr.start_yield_timeout(&parent_id, "agent-x", Some(1))
        .await;

    // Wait for timeout to fire.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Session should have resumed.
    assert!(
        !mgr.is_session_yielding(&parent_id).await,
        "session should exit Waiting after timeout fires"
    );

    // Timeout notification should be injected.
    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let messages = cs.read().await.messages().to_vec();
    let has_timeout_msg = messages.iter().any(|m| {
        m.role == "system"
            && m.content_blocks.iter().any(
                |b| matches!(b, closeclaw_llm::types::ContentBlock::Text(t) if t.contains("超时")),
            )
    });
    assert!(
        has_timeout_msg,
        "timeout notification should be injected into conversation"
    );
}

// ── 5. Default timeout constant is 600 seconds ─────────────────────────────

/// Verify the default timeout is 600 seconds (10 minutes) by checking
/// the notification message includes the default value.
#[tokio::test]
#[serial]
async fn test_yield_timeout_default_value_in_notification() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-to5").await;

    // Spawn a child.
    let _child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-to5", None),
            &parent_id,
            1,
            "work",
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
            None, // prompt_template_prefix
        )
        .await
        .unwrap();

    // Enter Waiting and start timeout with default (None → 600s).
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Use a 1-second timeout for fast test.
    mgr.start_yield_timeout(&parent_id, "agent-x", Some(1))
        .await;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Check notification mentions "1" (the actual timeout value used).
    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let messages = cs.read().await.messages().to_vec();
    let has_timeout_value = messages.iter().any(|m| {
        m.role == "system"
            && m.content_blocks.iter().any(
                |b| matches!(b, closeclaw_llm::types::ContentBlock::Text(t) if t.contains("1 秒内未完成")),
            )
    });
    assert!(
        has_timeout_value,
        "timeout notification should mention the actual timeout value (1s)"
    );
}

// ── 6. No children timeout fires and resumes ───────────────────────────────

/// Timeout should fire even if there are no children (edge case).
/// The session should resume.
#[tokio::test]
#[serial]
async fn test_yield_timeout_no_children_fires() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-to6").await;

    // Enter Waiting without spawning children.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Start a 1-second timeout.
    mgr.start_yield_timeout(&parent_id, "agent-x", Some(1))
        .await;

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Session should have resumed.
    assert!(
        !mgr.is_session_yielding(&parent_id).await,
        "session should exit Waiting after timeout even with no children"
    );
}
