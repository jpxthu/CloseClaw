//! Tests for notification priority display prefixes (must_fix 2, Step 1.2).
//!
//! Covers:
//! - Three-level prefixes (now/next/later) each appear in injected text
//! - `drain_and_inject_announces` injects prefix for each event
//! - `drain_and_inject_announces_filtered` injects prefix for each event
//! - `drain_announces_filtered` injects prefix for SystemNotification
//! - Same-level batch injection: each message gets a prefix
//! - Prefix does NOT appear on non-injected fields (e.g. AnnounceEvent itself)

use super::test_helpers::setup_parent_with_conv;
use super::tests::{clear_global_prompt_state, make_test_mgr};
use chrono::Utc;
use closeclaw_common::ChildCompletionStatus;
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::llm_session::{AnnounceEvent, ChatSession};
use closeclaw_tasks::NotificationPriority;
use tempfile::TempDir;

// ── Helpers ────────────────────────────────────────────────────────────────

/// Create an AnnounceEvent with the given priority and status.
fn make_event(
    child_id: &str,
    agent_id: &str,
    result: &str,
    priority: NotificationPriority,
    status: ChildCompletionStatus,
) -> AnnounceEvent {
    AnnounceEvent {
        child_session_id: child_id.into(),
        child_agent_id: agent_id.into(),
        result_text: result.into(),
        completed_at: Utc::now(),
        priority,
        status,
    }
}

/// Get injected system messages from a session's transcript.
async fn get_system_messages(mgr: &super::SessionManager, session_id: &str) -> Vec<String> {
    let cs = mgr.get_conversation_session(session_id).await.unwrap();
    let guard = cs.read().await;
    guard
        .messages()
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| {
            m.content_blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text(t) => Some(t.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .collect()
}

// ── drain_and_inject_announces: now prefix ─────────────────────────────────

/// AnnounceEvent with `Now` priority is injected with `[紧急]` prefix.
#[tokio::test]
async fn test_inject_now_prefix() {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let pid = "parent-prefix-now";
    setup_parent_with_conv(&mgr, pid).await;

    let event = make_event(
        "child-now",
        "agent-now",
        "urgent work done",
        NotificationPriority::Now,
        ChildCompletionStatus::Completed,
    );
    mgr.push_announce(&pid, event).await.unwrap();
    mgr.drain_and_inject_announces(&pid, None).await;

    let msgs = get_system_messages(&mgr, pid).await;
    assert_eq!(msgs.len(), 1, "should inject one system message");
    assert!(
        msgs[0].contains("[紧急]"),
        "Now prefix must contain [紧急], got: {}",
        msgs[0]
    );
    assert!(
        msgs[0].contains("urgent work done"),
        "result text must be present"
    );
    assert!(
        msgs[0].starts_with("[子 agent"),
        "must start with [子 agent ...], got: {}",
        msgs[0]
    );
}

// ── drain_and_inject_announces: next prefix ────────────────────────────────

/// AnnounceEvent with `Next` priority is injected with `[注意]` prefix.
#[tokio::test]
async fn test_inject_next_prefix() {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let pid = "parent-prefix-next";
    setup_parent_with_conv(&mgr, pid).await;

    let event = make_event(
        "child-next",
        "agent-next",
        "needs attention",
        NotificationPriority::Next,
        ChildCompletionStatus::Completed,
    );
    mgr.push_announce(&pid, event).await.unwrap();
    mgr.drain_and_inject_announces(&pid, None).await;

    let msgs = get_system_messages(&mgr, pid).await;
    assert_eq!(msgs.len(), 1);
    assert!(
        msgs[0].contains("[注意]"),
        "Next prefix must contain [注意], got: {}",
        msgs[0]
    );
    assert!(msgs[0].contains("needs attention"));
}

// ── drain_and_inject_announces: later prefix ───────────────────────────────

/// AnnounceEvent with `Later` priority is injected with `[后台]` prefix.
#[tokio::test]
async fn test_inject_later_prefix() {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let pid = "parent-prefix-later";
    setup_parent_with_conv(&mgr, pid).await;

    let event = make_event(
        "child-later",
        "agent-later",
        "background task done",
        NotificationPriority::Later,
        ChildCompletionStatus::Completed,
    );
    mgr.push_announce(&pid, event).await.unwrap();
    mgr.drain_and_inject_announces(&pid, None).await;

    let msgs = get_system_messages(&mgr, pid).await;
    assert_eq!(msgs.len(), 1);
    assert!(
        msgs[0].contains("[后台]"),
        "Later prefix must contain [后台], got: {}",
        msgs[0]
    );
    assert!(msgs[0].contains("background task done"));
}

// ── drain_and_inject_announces: mixed priorities ────────────────────────────

/// Multiple events with different priorities each get their own prefix.
#[tokio::test]
async fn test_inject_mixed_priority_prefixes() {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let pid = "parent-prefix-mix";
    setup_parent_with_conv(&mgr, pid).await;

    // Push events in reverse priority order (Later, Next, Now)
    // They will be drained in priority order: Now, Next, Later
    mgr.push_announce(
        &pid,
        make_event(
            "child-1",
            "agent-1",
            "urgent",
            NotificationPriority::Now,
            ChildCompletionStatus::Completed,
        ),
    )
    .await
    .unwrap();
    mgr.push_announce(
        &pid,
        make_event(
            "child-2",
            "agent-2",
            "attention",
            NotificationPriority::Next,
            ChildCompletionStatus::Completed,
        ),
    )
    .await
    .unwrap();
    mgr.push_announce(
        &pid,
        make_event(
            "child-3",
            "agent-3",
            "background",
            NotificationPriority::Later,
            ChildCompletionStatus::Completed,
        ),
    )
    .await
    .unwrap();

    mgr.drain_and_inject_announces(&pid, None).await;

    let msgs = get_system_messages(&mgr, pid).await;
    assert_eq!(msgs.len(), 3, "should inject 3 system messages");

    // Check each message has the correct prefix
    assert!(msgs[0].contains("[紧急]"), "first must be [紧急]");
    assert!(msgs[1].contains("[注意]"), "second must be [注意]");
    assert!(msgs[2].contains("[后台]"), "third must be [后台]");
}

// ── drain_and_inject_announces: same-level batch ───────────────────────────

/// Multiple events with the same priority each get a prefix.
#[tokio::test]
async fn test_inject_same_level_batch_each_has_prefix() {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let pid = "parent-prefix-batch";
    setup_parent_with_conv(&mgr, pid).await;

    // Push 3 events all with Now priority (dedup by child_session_id)
    mgr.push_announce(
        &pid,
        make_event(
            "batch-1",
            "agent-1",
            "result-1",
            NotificationPriority::Now,
            ChildCompletionStatus::Completed,
        ),
    )
    .await
    .unwrap();
    mgr.push_announce(
        &pid,
        make_event(
            "batch-2",
            "agent-2",
            "result-2",
            NotificationPriority::Now,
            ChildCompletionStatus::Completed,
        ),
    )
    .await
    .unwrap();
    mgr.push_announce(
        &pid,
        make_event(
            "batch-3",
            "agent-3",
            "result-3",
            NotificationPriority::Now,
            ChildCompletionStatus::Completed,
        ),
    )
    .await
    .unwrap();

    mgr.drain_and_inject_announces(&pid, None).await;

    let msgs = get_system_messages(&mgr, pid).await;
    assert_eq!(msgs.len(), 3, "should inject 3 messages");

    // Every message must contain [紧急]
    for (i, msg) in msgs.iter().enumerate() {
        assert!(
            msg.contains("[紧急]"),
            "message {} must contain [紧急] prefix, got: {}",
            i,
            msg
        );
    }
}

// ── drain_and_inject_announces_filtered: prefix per event ──────────────────

/// `drain_and_inject_announces_filtered` also adds prefixes.
#[tokio::test]
async fn test_filtered_inject_adds_prefix() {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let pid = "parent-prefix-filtered";
    setup_parent_with_conv(&mgr, pid).await;

    mgr.push_announce(
        &pid,
        make_event(
            "f-child-1",
            "f-agent-1",
            "filtered urgent",
            NotificationPriority::Now,
            ChildCompletionStatus::Completed,
        ),
    )
    .await
    .unwrap();
    mgr.push_announce(
        &pid,
        make_event(
            "f-child-2",
            "f-agent-2",
            "filtered next",
            NotificationPriority::Next,
            ChildCompletionStatus::Completed,
        ),
    )
    .await
    .unwrap();

    // Only drain Now priority
    mgr.drain_and_inject_announces_filtered(&pid, |p| matches!(p, NotificationPriority::Now), None)
        .await;

    let msgs = get_system_messages(&mgr, pid).await;
    assert_eq!(msgs.len(), 1, "only Now event should be injected");
    assert!(msgs[0].contains("[紧急]"), "must have [紧急] prefix");
    assert!(msgs[0].contains("filtered urgent"));

    // Next event should still be in queue
    let remaining = mgr.drain_announces(&pid).await;
    assert_eq!(remaining.len(), 1, "Next event should remain in queue");
    assert_eq!(remaining[0].child_session_id, "f-child-2");
}

// ── drain_announces_filtered: SystemNotification prefix ─────────────────────

/// `drain_announces_filtered` returns SystemNotification in DrainResult.system_notifications
/// with priority prefix, NOT injected into conversation transcript.
#[tokio::test]
async fn test_filtered_drain_system_notification_prefix() {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let pid = "parent-prefix-sys-notif";
    setup_parent_with_conv(&mgr, pid).await;

    // Push a system notification with Now priority
    {
        let cs = mgr.get_conversation_session(&pid).await.unwrap();
        let mut cs = cs.write().await;
        cs.push_system_notification("system urgent message".into(), NotificationPriority::Now);
    }

    // drain_announces_filtered with Now predicate should collect notification
    let drained = mgr
        .drain_announces_filtered(&pid, |p| matches!(p, NotificationPriority::Now))
        .await;

    // SystemNotification is NOT returned as AnnounceEvent
    assert!(
        drained.is_empty(),
        "SystemNotification should not appear in drained AnnounceEvents"
    );

    // SystemNotification should be in system_notifications with prefix
    assert_eq!(
        drained.system_notifications.len(),
        1,
        "system notification should be in DrainResult.system_notifications"
    );
    assert!(
        drained.system_notifications[0].contains("[紧急]"),
        "SystemNotification must have [紧急] prefix, got: {}",
        drained.system_notifications[0]
    );
    assert!(drained.system_notifications[0].contains("system urgent message"));

    // Notification should NOT be injected into conversation transcript
    let msgs = get_system_messages(&mgr, pid).await;
    assert_eq!(
        msgs.len(),
        0,
        "system notification should NOT be in transcript (routed outbound)"
    );
}

/// `drain_announces_filtered` with Next predicate returns SystemNotification
/// in DrainResult.system_notifications with [注意] prefix.
#[tokio::test]
async fn test_filtered_drain_system_notification_next_prefix() {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let pid = "parent-prefix-sys-next";
    setup_parent_with_conv(&mgr, pid).await;

    {
        let cs = mgr.get_conversation_session(&pid).await.unwrap();
        let mut cs = cs.write().await;
        cs.push_system_notification("next system msg".into(), NotificationPriority::Next);
    }

    let drained = mgr
        .drain_announces_filtered(&pid, |p| matches!(p, NotificationPriority::Next))
        .await;

    assert_eq!(
        drained.system_notifications.len(),
        1,
        "system notification should be in DrainResult.system_notifications"
    );
    assert!(
        drained.system_notifications[0].contains("[注意]"),
        "SystemNotification must have [注意] prefix, got: {}",
        drained.system_notifications[0]
    );

    // Should NOT be in transcript
    let msgs = get_system_messages(&mgr, pid).await;
    assert_eq!(
        msgs.len(),
        0,
        "system notification should NOT be in transcript (routed outbound)"
    );
}

// ── Errored status prefix ──────────────────────────────────────────────────

/// Errored status with Now priority still gets [紧急] prefix.
#[tokio::test]
async fn test_errored_status_gets_priority_prefix() {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let pid = "parent-prefix-err";
    setup_parent_with_conv(&mgr, pid).await;

    let event = make_event(
        "err-child",
        "err-agent",
        "error occurred",
        NotificationPriority::Now,
        ChildCompletionStatus::Errored,
    );
    mgr.push_announce(&pid, event).await.unwrap();
    mgr.drain_and_inject_announces(&pid, None).await;

    let msgs = get_system_messages(&mgr, pid).await;
    assert_eq!(msgs.len(), 1);
    assert!(
        msgs[0].contains("[紧急]"),
        "must have prefix even for Errored"
    );
    assert!(
        msgs[0].contains("任务出错"),
        "must contain Errored status label"
    );
}

/// Terminated status with Later priority gets [后台] prefix.
#[tokio::test]
async fn test_terminated_status_gets_priority_prefix() {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let pid = "parent-prefix-term";
    setup_parent_with_conv(&mgr, pid).await;

    let event = make_event(
        "term-child",
        "term-agent",
        "was terminated",
        NotificationPriority::Later,
        ChildCompletionStatus::Terminated,
    );
    mgr.push_announce(&pid, event).await.unwrap();
    mgr.drain_and_inject_announces(&pid, None).await;

    let msgs = get_system_messages(&mgr, pid).await;
    assert_eq!(msgs.len(), 1);
    assert!(
        msgs[0].contains("[后台]"),
        "must have prefix for Terminated"
    );
    assert!(
        msgs[0].contains("任务被终止"),
        "must contain Terminated status label"
    );
}

// ── Prefix format: [子 agent X] [prefix] status ────────────────────────────

/// Verify the full injected text format:
/// `[子 agent {id}] [{prefix}] {status}：\n{text}`
#[tokio::test]
async fn test_injected_text_format() {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let pid = "parent-prefix-format";
    setup_parent_with_conv(&mgr, pid).await;

    let event = make_event(
        "fmt-child",
        "fmt-agent",
        "format check",
        NotificationPriority::Next,
        ChildCompletionStatus::Completed,
    );
    mgr.push_announce(&pid, event).await.unwrap();
    mgr.drain_and_inject_announces(&pid, None).await;

    let msgs = get_system_messages(&mgr, pid).await;
    assert_eq!(msgs.len(), 1);
    let text = &msgs[0];

    // Full format check
    assert!(
        text.starts_with("[子 agent fmt-agent]"),
        "must start with [子 agent fmt-agent], got: {}",
        text
    );
    assert!(
        text.contains("[注意] 任务已完成："),
        "must contain [注意] 任务已完成：, got: {}",
        text
    );
    assert!(text.contains("format check"), "must contain result text");
}

// ── Non-matching events stay in queue ──────────────────────────────────────

/// `drain_announces_filtered` re-inserts non-matching events into queue.
#[tokio::test]
async fn test_filtered_drain_preserves_non_matching() {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let pid = "parent-prefix-preserve";
    setup_parent_with_conv(&mgr, pid).await;

    mgr.push_announce(
        &pid,
        make_event(
            "keep-child",
            "keep-agent",
            "keep this",
            NotificationPriority::Later,
            ChildCompletionStatus::Completed,
        ),
    )
    .await
    .unwrap();
    mgr.push_announce(
        &pid,
        make_event(
            "drain-child",
            "drain-agent",
            "drain this",
            NotificationPriority::Now,
            ChildCompletionStatus::Completed,
        ),
    )
    .await
    .unwrap();

    // Only drain Now
    let drained = mgr
        .drain_announces_filtered(&pid, |p| matches!(p, NotificationPriority::Now))
        .await;

    assert_eq!(drained.len(), 1, "should drain exactly 1 Now event");
    assert_eq!(drained[0].child_session_id, "drain-child");

    // Later event should still be in queue
    let remaining = mgr.drain_announces(&pid).await;
    assert_eq!(remaining.len(), 1, "Later event should remain");
    assert_eq!(remaining[0].child_session_id, "keep-child");
}

// ── Queue priority ordering preserved after drain ───────────────────────────

/// After `drain_and_inject_announces`, the injection order follows
/// priority: Now before Next before Later.
#[tokio::test]
async fn test_injection_order_follows_priority() {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let pid = "parent-prefix-order";
    setup_parent_with_conv(&mgr, pid).await;

    // Push in Later → Next → Now order (reverse of drain order)
    mgr.push_announce(
        &pid,
        make_event(
            "o-child-1",
            "o-agent-1",
            "later",
            NotificationPriority::Later,
            ChildCompletionStatus::Completed,
        ),
    )
    .await
    .unwrap();
    mgr.push_announce(
        &pid,
        make_event(
            "o-child-2",
            "o-agent-2",
            "next",
            NotificationPriority::Next,
            ChildCompletionStatus::Completed,
        ),
    )
    .await
    .unwrap();
    mgr.push_announce(
        &pid,
        make_event(
            "o-child-3",
            "o-agent-3",
            "now",
            NotificationPriority::Now,
            ChildCompletionStatus::Completed,
        ),
    )
    .await
    .unwrap();

    mgr.drain_and_inject_announces(&pid, None).await;

    let msgs = get_system_messages(&mgr, pid).await;
    assert_eq!(msgs.len(), 3);

    // Now (highest) → Next → Later (lowest)
    assert!(msgs[0].contains("[紧急]"), "first must be Now: {}", msgs[0]);
    assert!(
        msgs[1].contains("[注意]"),
        "second must be Next: {}",
        msgs[1]
    );
    assert!(
        msgs[2].contains("[后台]"),
        "third must be Later: {}",
        msgs[2]
    );
}

// ── SystemNotification outbound routing (Step 1.4) ─────────────────────────

/// Verify that `drain_announces` correctly separates `SystemNotification`
/// from `AnnounceEvent`: SystemNotification goes to `DrainResult.system_notifications`,
/// AnnounceEvent goes to `DrainResult.announces`.
#[tokio::test]
async fn test_drain_announces_separates_notifications_from_events() {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let pid = "parent-separate";
    setup_parent_with_conv(&mgr, pid).await;

    // Push an AnnounceEvent.
    mgr.push_announce(
        &pid,
        make_event(
            "child-sep",
            "agent-sep",
            "result text",
            NotificationPriority::Now,
            ChildCompletionStatus::Completed,
        ),
    )
    .await
    .unwrap();

    // Push a SystemNotification.
    {
        let cs = mgr.get_conversation_session(&pid).await.unwrap();
        let mut cs = cs.write().await;
        cs.push_system_notification("urgent system msg".into(), NotificationPriority::Now);
    }

    let result = mgr.drain_announces(&pid).await;

    // AnnounceEvent goes to announces.
    assert_eq!(result.announces.len(), 1);
    assert_eq!(result.announces[0].result_text, "result text");

    // SystemNotification goes to system_notifications.
    assert_eq!(result.system_notifications.len(), 1);
    assert!(result.system_notifications[0].contains("urgent system msg"));
}

/// `drain_and_inject_announces` with gateway=None does not panic.
/// SystemNotifications are silently dropped (warning logged).
#[tokio::test]
async fn test_drain_and_inject_gateway_none_no_panic() {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let pid = "parent-gw-none";
    setup_parent_with_conv(&mgr, pid).await;

    // Push a SystemNotification.
    {
        let cs = mgr.get_conversation_session(&pid).await.unwrap();
        let mut cs = cs.write().await;
        cs.push_system_notification("test notification".into(), NotificationPriority::Next);
    }

    // Call with gateway=None — should not panic.
    mgr.drain_and_inject_announces(&pid, None).await;

    // Notification should NOT be in transcript.
    let msgs = get_system_messages(&mgr, pid).await;
    assert!(
        msgs.is_empty(),
        "system notification should NOT be in transcript when gateway=None"
    );

    // Queue should be empty (drain consumed the entry).
    let cs = mgr.get_conversation_session(&pid).await.unwrap();
    assert!(cs.read().await.is_queue_empty());
}

/// `drain_and_inject_announces_filtered` with gateway=None does not panic.
#[tokio::test]
async fn test_drain_and_inject_filtered_gateway_none_no_panic() {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let pid = "parent-gw-none-filt";
    setup_parent_with_conv(&mgr, pid).await;

    // Push a SystemNotification and an AnnounceEvent.
    {
        let cs = mgr.get_conversation_session(&pid).await.unwrap();
        let mut cs = cs.write().await;
        cs.push_system_notification("test filtered".into(), NotificationPriority::Now);
    }
    mgr.push_announce(
        &pid,
        make_event(
            "child-filt",
            "agent-filt",
            "filtered result",
            NotificationPriority::Now,
            ChildCompletionStatus::Completed,
        ),
    )
    .await
    .unwrap();

    // Call with gateway=None.
    mgr.drain_and_inject_announces_filtered(&pid, |p| matches!(p, NotificationPriority::Now), None)
        .await;

    // AnnounceEvent should be injected as system message.
    let msgs = get_system_messages(&mgr, pid).await;
    assert_eq!(msgs.len(), 1);
    assert!(msgs[0].contains("filtered result"));

    // SystemNotification should NOT be in transcript (dropped, gateway=None).
    assert!(
        !msgs[0].contains("test filtered"),
        "system notification text should NOT appear in transcript"
    );

    // Queue should be empty.
    let cs = mgr.get_conversation_session(&pid).await.unwrap();
    assert!(cs.read().await.is_queue_empty());
}

/// Mixed queue: AnnounceEvent + SystemNotification. After drain_and_inject,
/// only AnnounceEvent appears in transcript; SystemNotification is routed outbound.
#[tokio::test]
async fn test_mixed_queue_only_announce_injected() {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let pid = "parent-mixed";
    setup_parent_with_conv(&mgr, pid).await;

    // Push AnnounceEvent (Now) and SystemNotification (Now).
    mgr.push_announce(
        &pid,
        make_event(
            "child-mix",
            "agent-mix",
            "announce result",
            NotificationPriority::Now,
            ChildCompletionStatus::Completed,
        ),
    )
    .await
    .unwrap();
    {
        let cs = mgr.get_conversation_session(&pid).await.unwrap();
        let mut cs = cs.write().await;
        cs.push_system_notification("system text".into(), NotificationPriority::Now);
    }

    mgr.drain_and_inject_announces(&pid, None).await;

    // Only AnnounceEvent should be in transcript.
    let msgs = get_system_messages(&mgr, pid).await;
    assert_eq!(msgs.len(), 1, "only announce event should be in transcript");
    assert!(msgs[0].contains("announce result"));
    assert!(
        !msgs[0].contains("system text"),
        "system notification should NOT be in transcript"
    );
}
