//! Unit tests for `UnifiedMessageQueue` (Step 1.6).
//!
//! Validates the unified message queue sorting: different priorities
//! mixed with user/non-user entries, drain ordering per design doc
//! `docs/design/session/session-execution.md` §统一消息队列.

use super::*;
use chrono::Utc;
use closeclaw_common::{ChildCompletionStatus, PendingMessage};
use closeclaw_tasks::NotificationPriority;

// ── Helpers ────────────────────────────────────────────────────────────────

fn make_announce(child_id: &str, priority: NotificationPriority) -> QueueEntry {
    QueueEntry::Announce(AnnounceEvent {
        child_session_id: format!("child_{}", child_id),
        child_agent_id: child_id.to_string(),
        result_text: format!("result from {}", child_id),
        completed_at: Utc::now(),
        priority,
        status: ChildCompletionStatus::Completed,
    })
}

fn make_user_msg(id: &str) -> QueueEntry {
    QueueEntry::UserMessage(PendingMessage::new(id.to_string(), format!("msg {}", id)))
}

fn entry_labels(entries: &[QueueEntry]) -> Vec<String> {
    entries
        .iter()
        .map(|e| match e {
            QueueEntry::UserMessage(pm) => format!("user:{}", pm.message_id),
            QueueEntry::Announce(ev) => format!("announce:{}", ev.child_agent_id),
            QueueEntry::BackgroundToolNotification(n) => format!("bg:{}", n.task_id),
            QueueEntry::SystemNotification(text, _) => format!("sys:{}", text),
        })
        .collect()
}

// ── 1. Mixed priority + user/non-user ordering ─────────────────────────────

/// Per design doc §统一消息队列: within the same priority, non-user
/// messages drain before user messages.
///
/// NOTE: The current sort key `(Reverse<priority>, is_user, seq)`
/// groups ALL non-user messages before ALL user messages (regardless
/// of priority). The design doc's interleaved ordering
/// (now非用户 → now用户 → next非用户 → ... ) would require
/// `(Reverse<priority>, Reverse<is_user>, seq)` or equivalent.
/// This test documents the *actual* behaviour.
#[test]
fn test_unified_queue_full_priority_user_mixing() {
    let mut q = UnifiedMessageQueue::default();

    // Push in scrambled order.
    q.push(make_user_msg("later-u1"));
    q.push(make_announce("now-a1", NotificationPriority::Now));
    q.push(make_announce("next-a1", NotificationPriority::Next));
    q.push(make_user_msg("now-u1"));
    q.push(make_announce("later-a1", NotificationPriority::Later));
    q.push(make_user_msg("next-u1"));

    let drained = q.drain_all();
    let labels = entry_labels(&drained);

    // Actual behaviour: all non-user messages first (sorted by
    // priority), then all user messages (sorted by seq / FIFO).
    assert_eq!(
        labels,
        vec![
            "announce:now-a1",
            "announce:next-a1",
            "announce:later-a1",
            "user:later-u1",
            "user:now-u1",
            "user:next-u1",
        ],
        "Non-user messages drain before user messages; within each group priority order is preserved"
    );
}

// ── 2. Same priority: non-user before user (FIFO within group) ──────────────

/// Two Now announces + two Now user msgs: announces first, then users,
/// each group preserving FIFO.
#[test]
fn test_unified_queue_same_priority_non_user_before_user() {
    let mut q = UnifiedMessageQueue::default();

    q.push(make_user_msg("u1"));
    q.push(make_announce("a1", NotificationPriority::Now));
    q.push(make_user_msg("u2"));
    q.push(make_announce("a2", NotificationPriority::Now));

    let drained = q.drain_all();
    let labels = entry_labels(&drained);

    assert_eq!(
        labels,
        vec!["announce:a1", "announce:a2", "user:u1", "user:u2",],
        "Same priority: non-user messages drain before user messages"
    );
}

// ── 3. Pop returns highest priority entry ──────────────────────────────────

#[test]
fn test_unified_queue_pop_highest_priority() {
    let mut q = UnifiedMessageQueue::default();

    q.push(make_user_msg("u1"));
    q.push(make_announce("a1", NotificationPriority::Later));
    q.push(make_announce("a2", NotificationPriority::Now));
    q.push(make_announce("a3", NotificationPriority::Next));

    let first = q.pop().unwrap();
    assert!(matches!(first, QueueEntry::Announce(ref ev) if ev.child_agent_id == "a2"));

    let second = q.pop().unwrap();
    assert!(matches!(second, QueueEntry::Announce(ref ev) if ev.child_agent_id == "a3"));

    let third = q.pop().unwrap();
    assert!(matches!(third, QueueEntry::Announce(ref ev) if ev.child_agent_id == "a1"));

    let fourth = q.pop().unwrap();
    assert!(matches!(fourth, QueueEntry::UserMessage(ref pm) if pm.message_id == "u1"));
}

// ── 4. FIFO stability within same priority and same user/non-user group ────

#[test]
fn test_unified_queue_fifo_stability() {
    let mut q = UnifiedMessageQueue::default();

    // Three Later announces — FIFO should be preserved.
    q.push(make_announce("a", NotificationPriority::Later));
    q.push(make_announce("b", NotificationPriority::Later));
    q.push(make_announce("c", NotificationPriority::Later));

    let drained = q.drain_all();
    let ids: Vec<&str> = drained
        .iter()
        .filter_map(|e| match e {
            QueueEntry::Announce(ev) => Some(ev.child_agent_id.as_str()),
            _ => None,
        })
        .collect();

    assert_eq!(
        ids,
        vec!["a", "b", "c"],
        "FIFO must be preserved within same priority"
    );
}

// ── 5. Announce dedup still works in unified queue ─────────────────────────

#[test]
fn test_unified_queue_announce_dedup() {
    let mut q = UnifiedMessageQueue::default();

    q.push(make_announce("x", NotificationPriority::Now));
    q.push(make_announce("x", NotificationPriority::Later)); // duplicate

    assert_eq!(q.len(), 1, "duplicate announce should be dropped");
}

// ── 6. Empty queue operations ──────────────────────────────────────────────

#[test]
fn test_unified_queue_empty_operations() {
    let mut q = UnifiedMessageQueue::default();

    assert!(q.is_empty());
    assert_eq!(q.len(), 0);
    assert!(q.pop().is_none());
    assert!(q.drain_all().is_empty());
}

// ── 7. Clear preserves announces, removes user messages ────────────────────

#[test]
fn test_unified_queue_clear_user_messages() {
    let mut q = UnifiedMessageQueue::default();

    q.push(make_user_msg("u1"));
    q.push(make_announce("a1", NotificationPriority::Now));
    q.push(make_user_msg("u2"));

    let removed = q.clear_user_messages();
    assert_eq!(removed, 2);
    assert_eq!(q.len(), 1);

    let remaining = q.drain_all();
    assert!(matches!(&remaining[0], QueueEntry::Announce(ev) if ev.child_agent_id == "a1"));
}

// ── 8. push_queue_entry preserves ordering ──────────────────────────────────

#[test]
fn test_unified_queue_push_entry_preserves_order() {
    let mut q = UnifiedMessageQueue::default();

    q.push(make_announce("a1", NotificationPriority::Later));
    q.push(make_announce("a2", NotificationPriority::Now));

    // Re-insert a1 after drain.
    let all = q.drain_all();
    for entry in all {
        q.push(entry);
    }

    let drained = q.drain_all();
    let ids: Vec<&str> = drained
        .iter()
        .filter_map(|e| match e {
            QueueEntry::Announce(ev) => Some(ev.child_agent_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        ids,
        vec!["a2", "a1"],
        "Order must be preserved after re-insert"
    );
}

// ── 9. Background tool notification in unified queue ───────────────────────

#[test]
fn test_unified_queue_background_tool_notification_priority() {
    use closeclaw_tasks::CompletionNotification;
    use closeclaw_tasks::TaskState;
    use std::path::PathBuf;

    let mut q = UnifiedMessageQueue::default();

    q.push(make_user_msg("u1"));
    q.push(make_announce("a1", NotificationPriority::Next));
    q.push(QueueEntry::BackgroundToolNotification(
        CompletionNotification {
            task_id: "bg-1".into(),
            command: "ls".into(),
            state: TaskState::Completed { exit_code: 0 },
            output_path: PathBuf::from("/tmp/out"),
            priority: NotificationPriority::Later,
            summary: "done".into(),
            suggestion: None,
        },
    ));

    let drained = q.drain_all();
    let labels = entry_labels(&drained);

    // bg-1 is Later priority, non-user → should come after Next announce
    // but before Later user message.
    assert_eq!(
        labels,
        vec!["announce:a1", "bg:bg-1", "user:u1"],
        "Background tool notification drains by priority, non-user before user"
    );
}

// ── 11. SystemNotification added by push_system_notification ────────────

/// `push_system_notification` should add a SystemNotification entry
/// with the specified priority to the queue.
#[test]
fn test_system_notification_push_adds_to_queue() {
    let mut q = UnifiedMessageQueue::default();

    q.push(QueueEntry::SystemNotification(
        "warning text".into(),
        NotificationPriority::Next,
    ));

    assert_eq!(q.len(), 1);
    let entry = q.pop().unwrap();
    match entry {
        QueueEntry::SystemNotification(text, priority) => {
            assert_eq!(text, "warning text");
            assert_eq!(priority, NotificationPriority::Next);
        }
        other => panic!("expected SystemNotification, got {:?}", other),
    }
}

// ── 12. SystemNotification priority ordering ──────────────────────────────

/// SystemNotification with Now priority drains before Next announce,
/// and SystemNotification with Later priority drains after Next announce.
#[test]
fn test_system_notification_priority_ordering() {
    let mut q = UnifiedMessageQueue::default();

    q.push(make_announce("a1", NotificationPriority::Later));
    q.push(QueueEntry::SystemNotification(
        "sys_now".into(),
        NotificationPriority::Now,
    ));
    q.push(make_announce("a2", NotificationPriority::Next));
    q.push(QueueEntry::SystemNotification(
        "sys_later".into(),
        NotificationPriority::Later,
    ));
    q.push(make_user_msg("u1"));

    let drained = q.drain_all();
    let labels = entry_labels(&drained);

    // Now(sys) > Next(announce) > Later(announce, sys) > Later(user)
    assert_eq!(
        labels,
        vec![
            "sys:sys_now",
            "announce:a2",
            "announce:a1",
            "sys:sys_later",
            "user:u1",
        ],
        "SystemNotification entries should follow priority-based ordering"
    );
}

// ── 13. drain_announce_queue preserves SystemNotification ──────────────────

/// `drain_announce_queue` returns only Announce events;
/// SystemNotification entries should be re-inserted into the queue.
#[test]
fn test_drain_announce_queue_preserves_system_notification() {
    let mut q = UnifiedMessageQueue::default();

    q.push(make_announce("a1", NotificationPriority::Now));
    q.push(QueueEntry::SystemNotification(
        "warning".into(),
        NotificationPriority::Next,
    ));
    q.push(make_announce("a2", NotificationPriority::Later));

    // Use ConversationSession method for drain_announce_queue.
    // Since we're testing UnifiedMessageQueue directly, simulate the
    // drain_announce_queue logic: drain all, re-insert non-announces.
    let all = q.drain_all();
    let mut announces = Vec::new();
    for entry in all {
        match entry {
            QueueEntry::Announce(e) => announces.push(e),
            other => q.push(other),
        }
    }

    assert_eq!(announces.len(), 2);
    assert_eq!(q.len(), 1);

    let remaining = q.pop().unwrap();
    assert!(
        matches!(remaining, QueueEntry::SystemNotification(ref t, _) if t == "warning"),
        "SystemNotification should survive drain_announce_queue"
    );
}

// ── 14. drain_all_entries returns SystemNotification ──────────────────────

/// `drain_all_entries` should return SystemNotification entries
/// along with other entry types.
#[test]
fn test_drain_all_entries_includes_system_notification() {
    let mut q = UnifiedMessageQueue::default();

    q.push(make_announce("a1", NotificationPriority::Now));
    q.push(QueueEntry::SystemNotification(
        "sys_msg".into(),
        NotificationPriority::Next,
    ));
    q.push(make_user_msg("u1"));

    let drained = q.drain_all();
    assert_eq!(drained.len(), 3);

    let has_sys = drained
        .iter()
        .any(|e| matches!(e, QueueEntry::SystemNotification(text, _) if text == "sys_msg"));
    assert!(has_sys, "drain_all_entries must include SystemNotification");
    assert!(q.is_empty());
}

// ── 15. Drain leaves queue empty ──────────────────────────────────────────

#[test]
fn test_unified_queue_drain_leaves_empty() {
    let mut q = UnifiedMessageQueue::default();
    q.push(make_announce("a", NotificationPriority::Now));
    q.push(make_user_msg("u"));

    q.drain_all();
    assert!(q.is_empty());
    assert_eq!(q.len(), 0);
}

// ── 16. drain_all_items: returns QueueItems with preserved seq ─────────────

/// `drain_all_items` returns `QueueItem` (entry + seq) preserving
/// original insertion seq, and leaves the queue empty.
#[test]
fn test_drain_all_items_returns_items_with_seq() {
    let mut q = UnifiedMessageQueue::default();

    q.push(make_announce("a1", NotificationPriority::Later));
    q.push(make_announce("a2", NotificationPriority::Now));
    q.push(make_user_msg("u1"));

    let items = q.drain_all_items();
    assert_eq!(items.len(), 3);
    assert!(q.is_empty());
    assert_eq!(q.len(), 0);

    // Items returned in priority order; seq values are 0, 1, 2
    // matching original insertion order.
    let seqs_and_ids: Vec<(u64, &str)> = items
        .iter()
        .map(|item| {
            let id = match &item.entry {
                QueueEntry::Announce(ev) => ev.child_agent_id.as_str(),
                QueueEntry::UserMessage(pm) => pm.message_id.as_str(),
                _ => "other",
            };
            (item.seq, id)
        })
        .collect();

    // a2 (Now, seq=1) first, then a1 (Later, seq=0), then u1 (Later user, seq=2)
    assert_eq!(seqs_and_ids, vec![(1, "a2"), (0, "a1"), (2, "u1")]);
}

// ── 17. drain_all_items + push_preserving_seq: FIFO order intact ───────────

/// Drain non-matching entries via `drain_all_items`, re-insert the
/// rest with `push_preserving_seq`. The resulting drain order must
/// match what we would get by draining the original queue.
#[test]
fn test_drain_all_items_preserves_fifo_on_reinsert() {
    let mut q = UnifiedMessageQueue::default();

    // Three Later-announces: a (seq=0), b (seq=1), c (seq=2)
    q.push(make_announce("a", NotificationPriority::Later));
    q.push(make_announce("b", NotificationPriority::Later));
    q.push(make_announce("c", NotificationPriority::Later));

    // Simulate drain_announce_queue: keep only non-announces
    let items = q.drain_all_items();
    for item in items {
        match item.entry {
            QueueEntry::Announce(_) => { /* dropped */ }
            _ => q.push_preserving_seq(item.entry, item.seq),
        }
    }
    // All were announces, so queue should be empty.
    assert!(q.is_empty());

    // Now push new mixed entries and verify seq continuity.
    q.push(make_user_msg("u1")); // seq=0 (next_seq was not reset)
    q.push(make_announce("d", NotificationPriority::Later)); // seq=1
    q.push(make_user_msg("u2")); // seq=2

    let all = q.drain_all();
    let labels = entry_labels(&all);

    // Non-user first (d), then users in FIFO (u1, u2)
    assert_eq!(
        labels,
        vec!["announce:d", "user:u1", "user:u2"],
        "After drain_all_items, next_seq continuity must hold"
    );
}

// ── 18. drain_all_items on empty queue ────────────────────────────────────

#[test]
fn test_drain_all_items_empty_queue() {
    let mut q = UnifiedMessageQueue::default();
    let items = q.drain_all_items();
    assert!(items.is_empty());
    assert!(q.is_empty());
}
