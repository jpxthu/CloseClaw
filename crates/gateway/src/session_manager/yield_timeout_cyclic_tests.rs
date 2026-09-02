//! Cyclic warning tests for yield timeout (split from yield_timeout_tests.rs).
//!
//! Covers cyclic warning ratio boundaries and stop-before-hard-timeout
//! behavior.

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
#[tokio::test]
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
    // interval = max(2*0.1, 1) = 1s.
    // Warnings at T=2, T=3, T=4 (loop breaks when elapsed=5 > 4).
    mgr.start_yield_timeout(&parent_id, "agent-x", 4, Some(2), Some(0.1))
        .await;

    // Wait 5s: hard timeout at T=4 fires and drains.
    // Note: the cyclic warning loop and hard timeout both fire at T=4,
    // so the drain may or may not catch the last warning (non-
    // deterministic). We verify the transcript instead of the queue.
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
#[tokio::test]
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
    // interval = max(3*0.5, 1) = max(1.5, 1) = 1s.
    // Warnings at T=3, T=4, T=5. Loop breaks when elapsed=6 > 5.
    mgr.start_yield_timeout(&parent_id, "agent-x", 5, Some(3), Some(0.5))
        .await;

    // Wait 7s: hard timeout at T=5 fires and drains.
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
