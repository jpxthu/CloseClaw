//! Cyclic warning tests for yield timeout (split from yield_timeout_tests.rs).
//!
//! Covers cyclic warning ratio boundaries and stop-before-hard-timeout
//! behavior.
//!
//! Timing-sensitive tests here run on a paused tokio clock
//! (`#[tokio::test(start_paused = true)]`): every `tokio::time::sleep`
//! inside `start_yield_timeout` — the cyclic warning timer and the hard
//! timeout timer — is registered on and fired by the virtual clock, so
//! the full timing chain is genuinely executed while wall-clock cost
//! collapses to milliseconds.
//!
//! `#[serial]` is unrelated to the virtual clock: it survives from the
//! era of the real global section cache (its clearing helper,
//! `clear_global_prompt_state`, is now an explicit no-op) and also
//! serializes access to the process-wide `SHARED_CONFIG_DIR` tempdir
//! shared by `make_test_mgr`.
//!
//! Note: test 17 (`test_yield_cyclic_warning_ratio_2_0_boundary`) still
//! uses the real clock; it is slated for the same paused-clock treatment
//! in a later batch.

use super::spawn::SpawnMode;
use super::test_helpers::{setup_parent_with_conv, test_resolved_config};
use super::tests::{clear_global_prompt_state, make_test_mgr};
use closeclaw_session::llm_session::ChatSession;
use closeclaw_tasks::NotificationPriority;
use serial_test::serial;
use std::sync::Arc;

// ── 16. ratio = 0.1 boundary (minimum) — interval clamped to 1s ──────────

/// When `notify_interval_ratio = 0.1`, the interval is
/// `warning_secs * 0.1`, which may be < 1s. The implementation clamps
/// to `max(interval, 1)` so warnings still fire at a 1-second cadence.
///
/// Runs on a paused tokio clock: the 4s hard timeout and the
/// 1s-cadence cyclic warning sleeps are virtual, so the full chain —
/// first warning at T=2, one more at T=3 (two in total), hard timeout
/// at T=4, drain completes — executes deterministically in
/// milliseconds of wall time.
#[tokio::test(start_paused = true)]
#[serial]
async fn test_yield_cyclic_warning_ratio_0_1_boundary() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-r01").await;

    let _child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-r01", None),
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
            None,
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // overall=4s, warning_secs=2s, ratio=0.1
    // interval = max(round(2*0.1), 1) = max(0, 1) = 1s.
    // Warnings at T=2 and T=3; after the T=3 injection elapsed=4 >= 4,
    // so the loop breaks without a further sleep. Hard timeout at T=4.
    mgr.start_yield_timeout(&parent_id, "agent-x", 4, Some(2), Some(0.1))
        .await;

    // Under the paused clock this 5s virtual sleep auto-advances
    // through the pending timers in order (warnings at T=2/3, hard
    // timeout at T=4) and returns only after the hard timeout fired
    // and drained — deterministic, ms-level wall-clock.
    // The last warning is enqueued at T=3, before the T=4 drain, so
    // the queue is deterministically empty afterwards; the transcript
    // scan below acts as the routing check.
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();

    // Notifications should have been queued and drained (routed outbound).
    // Queue should be empty after drain.
    assert!(
        cs.read().await.is_queue_empty(),
        "queue should be empty after timeout drain"
    );

    // Should NOT be in transcript (routed outbound).
    let messages = cs.read().await.messages().to_vec();
    let in_transcript = messages.iter().any(|m| {
        m.role == "system"
            && m.content_blocks.iter().any(|b| {
                matches!(b, closeclaw_llm::types::ContentBlock::Text(t)
                    if t.contains("等待上限") || t.contains("超时预警"))
            })
    });
    assert!(
        !in_transcript,
        "notifications should NOT be in transcript (routed outbound)"
    );

    // Verify drain_announces routes system notifications correctly.
    {
        let mut cs_write = cs.write().await;
        cs_write.push_system_notification(
            "[超时] verify routed outbound".into(),
            NotificationPriority::Next,
        );
        cs_write.push_system_notification(
            "[超时预警] verify warning routed".into(),
            NotificationPriority::Next,
        );
    }
    let drained = mgr.drain_announces(&parent_id).await;
    assert_eq!(drained.system_notifications.len(), 2);

    mgr.cancel_yield_timeout(&parent_id).await;
}

// ── 17. ratio = 2.0 boundary (maximum) — only initial warning fires ───────

/// When `notify_interval_ratio = 2.0`, the interval equals
/// `warning_secs * 2.0`. After the initial warning, elapsed exceeds
/// overall timeout so the loop breaks immediately — only one warning.
#[tokio::test]
#[serial]
async fn test_yield_cyclic_warning_ratio_2_0_boundary() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-r20").await;

    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // overall=3s, warning_secs=1s, ratio=2.0
    // interval = max(1*2, 1) = 2s.
    // Warning at T=1, elapsed=3 >= 3, loop breaks. One warning only.
    mgr.start_yield_timeout(&parent_id, "agent-x", 3, Some(1), Some(2.0))
        .await;

    // Wait 4s: hard timeout at T=3 fires, drain picks up warn at T=1.
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;

    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();

    // Queue should be empty after drain consumed entries.
    assert!(
        cs.read().await.is_queue_empty(),
        "queue should be empty after drain"
    );

    // Notifications should NOT be in transcript (routed outbound).
    let messages = cs.read().await.messages().to_vec();
    let in_transcript = messages.iter().any(|m| {
        m.role == "system"
            && m.content_blocks.iter().any(|b| {
                matches!(b, closeclaw_llm::types::ContentBlock::Text(t)
                    if t.contains("等待上限") || t.contains("超时预警"))
            })
    });
    assert!(
        !in_transcript,
        "notifications should NOT be in transcript (routed outbound)"
    );

    // Verify drain_announces routes system notifications correctly.
    {
        let mut cs_write = cs.write().await;
        cs_write.push_system_notification(
            "[超时] verify cyclic routed".into(),
            NotificationPriority::Next,
        );
    }
    let drained = mgr.drain_announces(&parent_id).await;
    assert_eq!(drained.system_notifications.len(), 1);
}

// ── 18. Cyclic warnings stop before hard timeout fires ────────────────────

/// Verify that the cyclic warning loop terminates before the hard
/// timeout — no warning is sent after `elapsed >= overall_timeout_secs`.
/// After hard timeout fires and drains, the queue contains only
/// warnings that were enqueued before the drain ran.
///
/// Runs on a paused tokio clock: the 5s hard timeout and the
/// 2s-interval cyclic warning sleeps are virtual, so the full chain —
/// the single warning at T=3 (the loop exits right after that
/// injection, when elapsed reaches the hard timeout), hard timeout at
/// T=5, drain completes — executes deterministically in milliseconds
/// of wall time.
#[tokio::test(start_paused = true)]
#[serial]
async fn test_yield_cyclic_warnings_stop_before_hard_timeout() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-cs").await;

    let _child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-cs", None),
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
            None,
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // overall=5s, warning_secs=3s, ratio=0.5
    // interval = max(round(3*0.5), 1) = max(2, 1) = 2s.
    // Single warning at T=3; after the injection elapsed=5 >= 5, so
    // the loop breaks without a further sleep — no warning at or after
    // the hard timeout. Hard timeout at T=5.
    mgr.start_yield_timeout(&parent_id, "agent-x", 5, Some(3), Some(0.5))
        .await;

    // Under the paused clock this 7s virtual sleep auto-advances
    // through the pending timers in order (warning at T=3, hard
    // timeout at T=5) and returns only after the hard timeout fired
    // and drained — deterministic, ms-level wall-clock. The 2s margin
    // beyond T=5 guarantees the hard-timeout callback (the only timer
    // left by then) is fully polled before the assertions below.
    tokio::time::sleep(std::time::Duration::from_secs(7)).await;

    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();

    // Notifications should have been queued and drained (routed outbound).
    // Queue should be empty after drain.
    assert!(
        cs.read().await.is_queue_empty(),
        "queue should be empty after drain"
    );

    // Should NOT be in transcript (routed outbound).
    let messages = cs.read().await.messages().to_vec();
    let in_transcript = messages.iter().any(|m| {
        m.role == "system"
            && m.content_blocks.iter().any(|b| {
                matches!(b, closeclaw_llm::types::ContentBlock::Text(t)
                    if t.contains("等待上限") || t.contains("超时预警"))
            })
    });
    assert!(
        !in_transcript,
        "notifications should NOT be in transcript (routed outbound)"
    );

    // Verify drain_announces routes system notifications correctly.
    {
        let mut cs_write = cs.write().await;
        cs_write.push_system_notification(
            "[超时] verify cyclic stop routed".into(),
            NotificationPriority::Next,
        );
    }
    let drained = mgr.drain_announces(&parent_id).await;
    assert_eq!(drained.system_notifications.len(), 1);
}
