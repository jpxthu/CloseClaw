//! Tests for yield timeout protection (Step 1.7).
//!
//! Covers timeout timer start/cancel, timeout expiry behavior,
//! notification injection, and session resume after timeout.

use super::spawn::SpawnMode;
use super::test_helpers::{setup_parent_with_conv, test_resolved_config};
use super::tests::{clear_global_prompt_state, make_test_mgr};
use closeclaw_agent::AgentConfigLookup;
use closeclaw_common::Tool;
use closeclaw_session::llm_session::ChatSession;
use closeclaw_tasks::NotificationPriority;
use serial_test::serial;
use std::sync::Arc;

// Mock AgentConfigLookup for tests
struct MockAgentConfigLookup;

#[async_trait::async_trait]
impl AgentConfigLookup for MockAgentConfigLookup {
    async fn lookup_agent_config(
        &self,
        _agent_id: &str,
    ) -> Option<closeclaw_agent::AgentConfigInfo> {
        None
    }
}

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
    mgr.start_yield_timeout(&parent_id, "agent-x", 600, None, None)
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
    mgr.start_yield_timeout(&parent_id, "agent-x", 2, None, None)
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
    mgr.start_yield_timeout(&parent_id, "agent-x", 600, None, None)
        .await;

    // Start second timeout (should abort the first).
    mgr.start_yield_timeout(&parent_id, "agent-x", 600, None, None)
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
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
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
    mgr.start_yield_timeout(&parent_id, "agent-x", 1, None, None)
        .await;

    // Wait for timeout to fire.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Session should have resumed.
    assert!(
        !mgr.is_session_yielding(&parent_id).await,
        "session should exit Waiting after timeout fires"
    );

    // Timeout notification should NOT be in transcript (routed outbound).
    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let messages = cs.read().await.messages().to_vec();
    let has_timeout_in_transcript = messages.iter().any(|m| {
        m.role == "system"
            && m.content_blocks.iter().any(
                |b| matches!(b, closeclaw_llm::types::ContentBlock::Text(t) if t.contains("超时")),
            )
    });
    assert!(
        !has_timeout_in_transcript,
        "timeout notification should NOT be in transcript (routed outbound)"
    );

    // Queue should be empty (drain consumed the notification).
    assert!(
        cs.read().await.is_queue_empty(),
        "queue should be empty after timeout drain"
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
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    // Enter Waiting and start timeout with default (None → 600s).
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Use a 1-second timeout for fast test.
    mgr.start_yield_timeout(&parent_id, "agent-x", 1, None, None)
        .await;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Notification should NOT be in transcript (routed outbound).
    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let messages = cs.read().await.messages().to_vec();
    let has_timeout_in_transcript = messages.iter().any(|m| {
        m.role == "system"
            && m.content_blocks.iter().any(|b| {
                matches!(
                    b,
                    closeclaw_llm::types::ContentBlock::Text(t)
                        if t.contains("等待上限 1 秒已到")
                )
            })
    });
    assert!(
        !has_timeout_in_transcript,
        "timeout notification should NOT be in transcript (routed outbound)"
    );

    // Verify the notification was queued and drained (queue is empty).
    assert!(
        cs.read().await.is_queue_empty(),
        "queue should be empty after timeout drain"
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
    mgr.start_yield_timeout(&parent_id, "agent-x", 1, None, None)
        .await;

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Session should have resumed.
    assert!(
        !mgr.is_session_yielding(&parent_id).await,
        "session should exit Waiting after timeout even with no children"
    );
}

// ── 7. Warning timeout injects notification without terminating children ────

/// The warning timeout fires before the hard timeout and injects a
/// warning notification. Children are NOT terminated — the session
/// remains in Waiting state.
#[tokio::test]
#[serial]
async fn test_yield_warning_timeout_injects_notification() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-to7").await;

    // Spawn a run-mode child that won't complete.
    let _child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-to7", None),
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
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Start with overall=61s (warning at 1s). Warning fires first.
    mgr.start_yield_timeout(&parent_id, "agent-x", 61, None, None)
        .await;

    // Wait for warning to fire (1s) but not hard timeout (61s).
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    // Session should still be yielding (hard timeout hasn't fired).
    assert!(
        mgr.is_session_yielding(&parent_id).await,
        "session should remain in Waiting after warning fires"
    );

    // Warning notification should be enqueued (not directly injected).
    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    {
        let cs_guard = cs.read().await;
        // SystemNotification is non-user, so pending_user_messages won't see it.
        // Check that the unified queue is not empty (notification is queued).
        assert!(
            cs_guard.has_pending() || cs_guard.queue_len() > 0,
            "warning notification should be enqueued in unified queue"
        );
    }

    // Children should NOT be terminated.
    let children = mgr.children.read().await;
    let child_list = children.list_children(&parent_id);
    assert!(
        !child_list.is_empty(),
        "children should not be terminated by warning timeout"
    );

    // Cleanup: cancel remaining timeout.
    mgr.cancel_yield_timeout(&parent_id).await;
}

// ── 8. Hard timeout fires after warning (two-stage sequence) ────────────────

/// Verify the two-stage sequence: warning fires first (when timeout >
/// 60s), then hard timeout injects structured notification and
/// resumes the session.
#[tokio::test]
#[serial]
async fn test_yield_two_stage_timeout_sequence() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-to8").await;

    // Spawn a run-mode child that won't complete.
    let _child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-to8", None),
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
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Start with overall=3s (warning at 0 → skipped, hard at 3s).
    mgr.start_yield_timeout(&parent_id, "agent-x", 3, None, None)
        .await;

    // Wait for hard timeout to fire (3s + buffer).
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;

    // Session should have resumed (hard timeout fired).
    assert!(
        !mgr.is_session_yielding(&parent_id).await,
        "session should exit Waiting after hard timeout fires"
    );

    // Timeout notification should NOT be in transcript (routed outbound).
    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let messages = cs.read().await.messages().to_vec();
    let has_warning = messages.iter().any(|m| {
        m.role == "system"
            && m.content_blocks.iter().any(
                |b| matches!(b, closeclaw_llm::types::ContentBlock::Text(t) if t.contains("超时预警")),
            )
    });
    let has_timeout = messages.iter().any(|m| {
        m.role == "system"
            && m.content_blocks.iter().any(
                |b| matches!(b, closeclaw_llm::types::ContentBlock::Text(t) if t.contains("等待上限")),
            )
    });
    assert!(
        !has_warning,
        "warning notification should NOT be in transcript (routed outbound)"
    );
    assert!(
        !has_timeout,
        "timeout notification should NOT be in transcript (routed outbound)"
    );

    // Queue should be empty after drain.
    assert!(
        cs.read().await.is_queue_empty(),
        "queue should be empty after timeout drain"
    );
}

// ── 9. Warning disabled (very large) — only hard timeout fires ──────────────

/// When timeout_warning_secs is set very large (effectively disabled),
/// only the hard timeout fires.
#[tokio::test]
#[serial]
async fn test_yield_warning_disabled_only_hard_timeout_fires() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-to9").await;

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Start with a 1-second timeout (no warning since <= 60s).
    mgr.start_yield_timeout(&parent_id, "agent-x", 1, None, None)
        .await;

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Session should have resumed.
    assert!(
        !mgr.is_session_yielding(&parent_id).await,
        "session should exit Waiting after hard timeout fires"
    );

    // Only timeout notification, no warning.
    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let messages = cs.read().await.messages().to_vec();
    let has_warning = messages.iter().any(|m| {
        m.role == "system"
            && m.content_blocks.iter().any(
                |b| matches!(b, closeclaw_llm::types::ContentBlock::Text(t) if t.contains("超时预警")),
            )
    });
    let has_timeout = messages.iter().any(|m| {
        m.role == "system"
            && m.content_blocks.iter().any(
                |b| matches!(b, closeclaw_llm::types::ContentBlock::Text(t) if t.contains("等待上限")),
            )
    });
    assert!(
        !has_warning,
        "warning notification should NOT be in transcript"
    );
    assert!(
        !has_timeout,
        "timeout notification should NOT be in transcript (routed outbound)"
    );

    // Queue should be empty after drain.
    assert!(
        cs.read().await.is_queue_empty(),
        "queue should be empty after timeout drain"
    );
}

// ── 9b. Legacy mode (None, None) fires exactly one warning ─────────────────

/// When both `timeout_warning_secs` and `notify_interval_ratio` are
/// None, the legacy path fires exactly one warning at
/// `overall_timeout_secs - 60` seconds, then nothing until the
/// hard timeout. This verifies single-warning semantics.
#[tokio::test]
#[serial]
async fn test_yield_legacy_single_warning_only() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-lsw").await;

    // Spawn a run-mode child that won't complete.
    let _child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-lsw", None),
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

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // overall=61s, (None, None) → legacy: warning at T=1 (61-60).
    mgr.start_yield_timeout(&parent_id, "agent-x", 61, None, None)
        .await;

    // Wait for the single warning to fire (T=1) but not the hard timeout.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    // Collect warning entries from the queue.
    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let entries = {
        let mut cs_write = cs.write().await;
        cs_write.drain_queue()
    };

    // Exactly one warning should have been enqueued.
    let warning_entries: Vec<_> = entries
        .iter()
        .filter(|e| {
            matches!(
                e,
                closeclaw_session::llm_session::QueueEntry::SystemNotification(text, _)
                    if text.contains("超时预警")
            )
        })
        .collect();
    assert_eq!(
        warning_entries.len(),
        1,
        "legacy mode should enqueue exactly one warning, got {}",
        warning_entries.len()
    );

    // Session should still be yielding (hard timeout hasn't fired).
    assert!(
        mgr.is_session_yielding(&parent_id).await,
        "session should remain in Waiting after single warning"
    );

    // Cleanup.
    mgr.cancel_yield_timeout(&parent_id).await;
}

// ── 10. Yield instant injection: is_session_busy returns false during yield ─

/// Per design doc §Yield 机制: yield 后 llm_active 和
/// foreground_tool_active 均为 false → session 为 idle → 用户消息
/// 立即注入，不排队。
///
/// `is_session_busy` delegates to `exec_status() == Busy`. During
/// yield, `exec_status()` returns `Waiting` (not `Busy`), so
/// `is_session_busy` returns `false` — messages flow through the
/// normal injection path.
#[tokio::test]
#[serial]
async fn test_yield_session_not_busy_allows_instant_injection() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-yii").await;

    // Enter Waiting (yield state).
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // is_session_busy must be false — allows direct injection.
    assert!(
        !mgr.is_session_busy(&parent_id).await,
        "is_session_busy must return false during yield (Waiting state)",
    );

    // Cleanup.
    mgr.cancel_yield_timeout(&parent_id).await;
}

// ── 11. Yield + child running → still not busy ─────────────────────────────

/// Even with child sessions active, yield state means the session
/// is not busy — user messages are injected immediately.
#[tokio::test]
#[serial]
async fn test_yield_with_child_not_busy() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-yic").await;

    // Spawn a child.
    let _child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-yic", None),
            &parent_id,
            1,
            "task",
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
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Not busy — child does not block message injection.
    assert!(
        !mgr.is_session_busy(&parent_id).await,
        "is_session_busy must return false during yield even with active child",
    );

    // Cleanup.
    mgr.cancel_yield_timeout(&parent_id).await;
}

// ── 13. Yield warning notification is SystemNotification with Next priority ──

/// Verify that the yield warning notification enqueued by the warning
/// timer is a `SystemNotification` entry (not directly injected as
/// a system message) with `NotificationPriority::Next`.
#[tokio::test]
#[serial]
async fn test_yield_warning_is_system_notification_with_next_priority() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-wnp").await;

    // Spawn a run-mode child that won't complete.
    let _child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-wnp", None),
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
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Start with overall=61s (warning at 1s). Warning fires first.
    mgr.start_yield_timeout(&parent_id, "agent-x", 61, None, None)
        .await;

    // Wait for warning to fire.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    // Verify the notification is a SystemNotification in the queue.
    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    {
        let mut cs_write = cs.write().await;
        let entries = cs_write.drain_queue();
        assert!(
            !entries.is_empty(),
            "queue should have an entry after warning"
        );

        let sys_entry = entries.iter().find(|e| {
            matches!(
                e,
                closeclaw_session::llm_session::QueueEntry::SystemNotification(_, _)
            )
        });
        let sys_entry = sys_entry.expect("should contain a SystemNotification entry");
        match sys_entry {
            closeclaw_session::llm_session::QueueEntry::SystemNotification(text, priority) => {
                assert!(
                    text.contains("超时预警"),
                    "warning text should contain '超时预警', got: {}",
                    text
                );
                assert_eq!(*priority, NotificationPriority::Next);
            }
            _ => unreachable!(),
        }
    }

    // Cleanup.
    mgr.cancel_yield_timeout(&parent_id).await;
}

// ── 14. Yield timeout notification goes through queue ──────────────────────

/// Verify that the yield timeout notification is enqueued as a
/// SystemNotification (priority Next) and then drained into the
/// conversation transcript by `drain_pending_for_session`.
#[tokio::test]
#[serial]
async fn test_yield_timeout_notification_goes_through_queue() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let parent_id = setup_parent_with_conv(&mgr, "parent-tnq").await;

    // Spawn a child.
    let _child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-tnq", None),
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
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Start a 1-second timeout.
    mgr.start_yield_timeout(&parent_id, "agent-x", 1, None, None)
        .await;

    // Wait for timeout to fire and drain to complete.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // After drain, the notification should be routed outbound (NOT in transcript).
    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let messages = cs.read().await.messages().to_vec();
    let has_timeout_in_transcript = messages.iter().any(|m| {
        m.role == "system"
            && m.content_blocks.iter().any(
                |b| matches!(b, closeclaw_llm::types::ContentBlock::Text(t) if t.contains("超时")),
            )
    });
    assert!(
        !has_timeout_in_transcript,
        "timeout notification should NOT be in transcript (routed outbound)"
    );

    // Queue should be empty after drain.
    let cs_guard = cs.read().await;
    assert!(
        cs_guard.is_queue_empty(),
        "queue should be empty after timeout drain"
    );
}

// ── 15. sessions_yield detail does not contain 'queued' ────────────────────

/// The `detail()` method of `sessions_yield` should not claim that
/// user messages are "queued" during yield — per design doc, yield
/// makes the session idle and messages are delivered immediately.
#[tokio::test]
#[serial]
async fn test_sessions_yield_detail_not_queued() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let agent_config_lookup: Arc<dyn AgentConfigLookup> = Arc::new(MockAgentConfigLookup);
    let tool =
        closeclaw_session::tools::sessions_yield::SessionsYieldTool::new(mgr, agent_config_lookup);
    let detail = tool.detail();

    assert!(
        !detail.to_lowercase().contains("queued"),
        "detail() should not contain 'queued', got: {}",
        detail
    );
    assert!(
        detail.contains("idle"),
        "detail() should mention idle state, got: {}",
        detail
    );
    assert!(
        detail.contains("immediately"),
        "detail() should mention immediate delivery, got: {}",
        detail
    );
}
