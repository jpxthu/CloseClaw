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
//! Intermediate-state assertions use `advance_and_settle`: the paused
//! clock is moved stepwise to each expected firing moment, so warning
//! counts can be pinned to exact virtual times instead of only being
//! observable in the post-drain aggregate.
//!
//! `#[serial]` is unrelated to the virtual clock: it survives from the
//! era of the real global section cache (its clearing helper,
//! `clear_global_prompt_state`, is now an explicit no-op) and also
//! serializes access to the process-wide `SHARED_CONFIG_DIR` tempdir
//! shared by `make_test_mgr`.

use super::spawn::SpawnMode;
use super::test_helpers::{setup_parent_with_conv, test_spawn_target_config};
use super::tests::{clear_global_prompt_state, make_test_mgr};
use super::SessionManager;
use closeclaw_common::NotificationPriority;
use closeclaw_session::llm_session::ChatSession;
use serial_test::serial;
use std::sync::Arc;

/// Shared post-drain verification tail for the cyclic-warning tests.
///
/// Asserts, in order:
/// 1. the notification queue is empty after the hard-timeout drain;
/// 2. no timeout/warning notification appears in the transcript (they
///    were routed outbound instead);
/// 3. `drain_announces` routes system notifications: `texts` are pushed
///    onto the parent session, drained, and the drained count must
///    equal the pushed count.
async fn assert_routed_outbound(mgr: &SessionManager, parent_id: &str, texts: &[&str]) {
    let cs = mgr.get_conversation_session(parent_id).await.unwrap();

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
        for text in texts {
            cs_write.push_system_notification((*text).into(), NotificationPriority::Next);
        }
    }
    let drained = mgr.drain_announces(parent_id).await;
    assert_eq!(drained.system_notifications.len(), texts.len());
}

/// Move the paused clock forward by `d`, letting freshly spawned tasks
/// register their timers before the jump and the handlers woken by the
/// jump run to completion after it.
///
/// Two tokio details make both phases necessary:
/// - A `tokio::spawn`ed task registers its first `sleep` only when the
///   runtime first polls it — which may not have happened by the time
///   the spawning call returns. The pre-jump yields ensure every
///   pending spawn has parked on its timer, so `advance` actually
///   passes its deadline.
/// - `tokio::time::advance` fires every timer whose deadline the jump
///   passes but does not itself poll the tasks those timers wake. The
///   warning / hard-timeout handlers contain no timers of their own —
///   every internal `.await` is an uncontended tokio lock that resolves
///   on the first poll — so the post-jump yields deterministically run
///   each woken task to its next parking point (or exit). Should a
///   handler ever fail to settle within the bound, the count assertions
///   below fail loudly instead of hanging.
async fn advance_and_settle(d: std::time::Duration) {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(d).await;
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

/// Drain the parent's notification queue and return the texts of the
/// timeout-warning entries only (count template: yield_timeout_tests
/// "exactly one warning").
async fn drain_warnings(mgr: &SessionManager, parent_id: &str) -> Vec<String> {
    let cs = mgr.get_conversation_session(parent_id).await.unwrap();
    let entries = {
        let mut cs_write = cs.write().await;
        cs_write.drain_queue()
    };
    let mut warnings = Vec::new();
    for e in entries {
        if let closeclaw_session::llm_session::QueueEntry::SystemNotification(text, _, _) = e {
            if text.contains("超时预警") {
                warnings.push(text);
            }
        }
    }
    warnings
}

// ── 16. ratio = 0.1 boundary (minimum) — interval clamped to 1s ──────────

/// When `notify_interval_ratio = 0.1`, the interval is
/// `warning_secs * 0.1`, which may be < 1s. The implementation clamps
/// to `max(interval, 1)` so warnings still fire at a 1-second cadence.
///
/// Runs on a paused tokio clock with stepwise `advance`: the clock is
/// moved to each expected firing moment (T=2, T=3, T=4) and the queue
/// is drained at every checkpoint, so each warning is pinned to its
/// exact virtual time — one at T=2 and one more at T=3 (two in total) —
/// before the hard timeout at T=4 drains for outbound routing. The
/// whole chain executes deterministically in milliseconds of wall time.
#[tokio::test(start_paused = true)]
#[serial]
async fn test_yield_cyclic_warning_ratio_0_1_boundary() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-r01").await;

    let _child_id = mgr
        .create_child_session(
            &test_spawn_target_config("worker-r01", None),
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

    // Stepwise checkpoints pin each warning to its exact virtual
    // moment. T=2: exactly the first warning fires.
    advance_and_settle(std::time::Duration::from_secs(2)).await;
    let first = drain_warnings(&mgr, &parent_id).await;
    assert_eq!(
        first.len(),
        1,
        "exactly one warning should fire at T=2, got {}",
        first.len()
    );
    assert!(
        first[0].contains("已运行 2 秒"),
        "T=2 warning should report elapsed 2s, got: {}",
        first[0]
    );

    // T=3: exactly one more warning fires; the loop then breaks
    // (elapsed=4 >= 4), so no further warnings exist.
    advance_and_settle(std::time::Duration::from_secs(1)).await;
    let second = drain_warnings(&mgr, &parent_id).await;
    assert_eq!(
        second.len(),
        1,
        "exactly one more warning should fire at T=3, got {}",
        second.len()
    );
    assert!(
        second[0].contains("已运行 3 秒"),
        "T=3 warning should report elapsed 3s, got: {}",
        second[0]
    );

    // T=4: the hard timeout fires and drains for outbound routing —
    // deterministic, ms-level wall-clock.
    advance_and_settle(std::time::Duration::from_secs(1)).await;

    assert_routed_outbound(
        &mgr,
        &parent_id,
        &[
            "[超时] verify routed outbound",
            "[超时预警] verify warning routed",
        ],
    )
    .await;

    mgr.cancel_yield_timeout(&parent_id).await;
}

// ── 17. ratio = 2.0 boundary (maximum) — only initial warning fires ───────

/// When `notify_interval_ratio = 2.0`, the interval equals
/// `warning_secs * 2.0`. After the initial warning, elapsed exceeds
/// overall timeout so the loop breaks immediately — only one warning.
///
/// Runs on a paused tokio clock: the 3s hard timeout and the 2s
/// cyclic-warning interval sleeps are virtual, so the full chain —
/// the single warning at T=1 (the loop exits right after that
/// injection, when elapsed reaches the hard timeout), hard timeout at
/// T=3, drain completes — executes deterministically in milliseconds
/// of wall time.
#[tokio::test(start_paused = true)]
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
    // interval = max(round(1*2.0), 1) = 2s.
    // Warning at T=1, elapsed=3 >= 3, loop breaks. One warning only.
    mgr.start_yield_timeout(&parent_id, "agent-x", 3, Some(1), Some(2.0))
        .await;

    // Under the paused clock this 2s virtual sleep stops the clock at
    // T=2: the boundary warning (T=1) is already enqueued while the
    // hard timeout (T=3) has not fired yet — a deterministic pause
    // point for the positive count assertion below.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Positive assertion: exactly one entry — the T=1 boundary warning
    // — was enqueued before the hard-timeout drain. (Count template:
    // yield_timeout_tests "exactly one warning".)
    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let entries = {
        let mut cs_write = cs.write().await;
        cs_write.drain_queue()
    };
    let warning_entries: Vec<_> = entries
        .iter()
        .filter(|e| {
            matches!(
                e,
                closeclaw_session::llm_session::QueueEntry::SystemNotification(text, _, _)
                    if text.contains("超时预警")
            )
        })
        .collect();
    assert_eq!(
        warning_entries.len(),
        1,
        "ratio=2.0 boundary should enqueue exactly one warning before the \
         hard timeout, got {}",
        warning_entries.len()
    );

    // Advance through the hard timeout at T=3 (the 2s margin beyond
    // T=3 guarantees the hard-timeout callback is fully polled): it
    // fires and drains the queue for outbound routing — deterministic,
    // ms-level wall-clock.
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    assert_routed_outbound(&mgr, &parent_id, &["[超时] verify cyclic routed"]).await;
}

// ── 18. Cyclic warnings stop before hard timeout fires ────────────────────

/// Verify that the cyclic warning loop terminates before the hard
/// timeout — no warning is sent after `elapsed >= overall_timeout_secs`.
/// The stepwise checkpoints assert this property directly: exactly one
/// warning at T=3, the queue still empty just before the hard timeout
/// at T=5 (nothing fired in between), then the timeout drain.
///
/// Runs on a paused tokio clock with stepwise `advance`: the clock is
/// moved to T=3, to just before the hard timeout at T=5, then past it —
/// all deterministically in milliseconds of wall time.
#[tokio::test(start_paused = true)]
#[serial]
async fn test_yield_cyclic_warnings_stop_before_hard_timeout() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-cs").await;

    let _child_id = mgr
        .create_child_session(
            &test_spawn_target_config("worker-cs", None),
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

    // T=3: exactly one warning fires, then the loop breaks.
    advance_and_settle(std::time::Duration::from_secs(3)).await;
    let warnings = drain_warnings(&mgr, &parent_id).await;
    assert_eq!(
        warnings.len(),
        1,
        "exactly one warning should fire at T=3, got {}",
        warnings.len()
    );
    assert!(
        warnings[0].contains("已运行 3 秒"),
        "T=3 warning should report elapsed 3s, got: {}",
        warnings[0]
    );

    // T=3 → just before the hard timeout (T=5): the queue must stay
    // empty — the warning loop has stopped and nothing may fire until
    // the hard timeout itself. This turns "stop before hard timeout"
    // from a code-reading inference into a direct assertion.
    advance_and_settle(std::time::Duration::from_millis(1999)).await;
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        assert!(
            cs.read().await.is_queue_empty(),
            "no warning may appear between the last cyclic warning (T=3) \
             and the hard timeout (T=5)"
        );
    }

    // T=5: the hard timeout fires and drains for outbound routing —
    // deterministic, ms-level wall-clock.
    advance_and_settle(std::time::Duration::from_millis(1)).await;

    assert_routed_outbound(&mgr, &parent_id, &["[超时] verify cyclic stop routed"]).await;
}
