//! Tests for `UnifiedMessageQueue` — Step 1.1 and Step 1.5.
//!
//! Validates priority-ordered drain, user/non-user interleaving,
//! deduplication, background tool notification insertion, and
//! the unified drain contract.

use super::*;
use chrono::Utc;
use closeclaw_common::ChildCompletionStatus;
use closeclaw_tasks::NotificationPriority;

// ── Helpers ────────────────────────────────────────────────────────────────

fn make_announce(agent_id: &str, priority: NotificationPriority) -> AnnounceEvent {
    AnnounceEvent {
        child_session_id: format!("child_{}", agent_id),
        child_agent_id: agent_id.to_string(),
        result_text: format!("result from {}", agent_id),
        completed_at: Utc::now(),
        priority,
        status: ChildCompletionStatus::Completed,
    }
}

fn make_bg_notification(
    task_id: &str,
    priority: NotificationPriority,
) -> closeclaw_tasks::CompletionNotification {
    closeclaw_tasks::CompletionNotification {
        task_id: task_id.to_string(),
        command: "echo test".to_string(),
        state: closeclaw_tasks::TaskState::Completed { exit_code: 0 },
        output_path: std::path::PathBuf::from("/tmp/output"),
        priority,
        summary: format!("task {} done", task_id),
        suggestion: None,
    }
}

// ── 1. Mixed priority + user/non-user drain order ──────────────────────────

/// Push user (Later), Now announce, Next announce; drain must yield
/// Now → Next → User (later), because within the same priority
/// (Later), user messages rank lower than non-user messages.
#[test]
fn test_unified_queue_mixed_priority_user_vs_non_user() {
    let mut session = ConversationSession::new("uq_mix1".into(), "gpt-4o".into(), tmp_path());
    // User message is always Later priority.
    session.push_pending(PendingMessage::new("u1".into(), "hello".into()));
    session.push_announce_to_queue(make_announce("now_agent", NotificationPriority::Now));
    session.push_announce_to_queue(make_announce("next_agent", NotificationPriority::Next));

    let entries = session.drain_all_entries();
    let ids: Vec<&str> = entries
        .iter()
        .map(|e| match e {
            QueueEntry::Announce(a) => a.child_agent_id.as_str(),
            QueueEntry::UserMessage(_) => "user",
            QueueEntry::BackgroundToolNotification(_) => "bg",
        })
        .collect();
    // Now > Next > User(later)
    assert_eq!(ids, vec!["now_agent", "next_agent", "user"]);
}

/// Push two user messages and two Now announces; drain must yield
/// Now announces first (same priority, non-user beats user), then
/// user messages in FIFO order.
#[test]
fn test_unified_queue_same_priority_non_user_before_user() {
    let mut session = ConversationSession::new("uq_mix2".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_announce("n1", NotificationPriority::Now));
    session.push_pending(PendingMessage::new("u1".into(), "first".into()));
    session.push_announce_to_queue(make_announce("n2", NotificationPriority::Now));
    session.push_pending(PendingMessage::new("u2".into(), "second".into()));

    let entries = session.drain_all_entries();
    let ids: Vec<&str> = entries
        .iter()
        .map(|e| match e {
            QueueEntry::Announce(a) => a.child_agent_id.as_str(),
            QueueEntry::UserMessage(pm) => pm.message_id.as_str(),
            QueueEntry::BackgroundToolNotification(_) => "bg",
        })
        .collect();
    // Now non-user (n1, n2) first, then user (u1, u2)
    assert_eq!(ids, vec!["n1", "n2", "u1", "u2"]);
}

// ── 2. pop returns highest priority entry ──────────────────────────────────

/// pop() should return entries in strict priority order.
#[test]
fn test_unified_queue_pop_priority_order() {
    let mut session = ConversationSession::new("uq_pop1".into(), "gpt-4o".into(), tmp_path());
    session.push_pending(PendingMessage::new("u1".into(), "later".into()));
    session.push_announce_to_queue(make_announce("later_a", NotificationPriority::Later));
    session.push_announce_to_queue(make_announce("next_a", NotificationPriority::Next));
    session.push_announce_to_queue(make_announce("now_a", NotificationPriority::Now));

    let first = session.pop_queue_entry().unwrap();
    assert!(matches!(first, QueueEntry::Announce(a) if a.child_agent_id == "now_a"));

    let second = session.pop_queue_entry().unwrap();
    assert!(matches!(second, QueueEntry::Announce(a) if a.child_agent_id == "next_a"));
}

// ── 3. Dedup announce events by child_session_id ──────────────────────────

/// Pushing the same child_session_id twice should only keep one entry.
#[test]
fn test_unified_queue_dedup_announce_by_child_session_id() {
    let mut session = ConversationSession::new("uq_dedup".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_announce("a1", NotificationPriority::Now));
    session.push_announce_to_queue(make_announce("a1", NotificationPriority::Now));
    // Second push should be deduplicated.
    assert_eq!(session.queue_len(), 1);
}

/// Different child_session_ids should both be kept.
#[test]
fn test_unified_queue_dedup_different_children_allowed() {
    let mut session = ConversationSession::new("uq_dedup2".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_announce("a1", NotificationPriority::Now));
    session.push_announce_to_queue(make_announce("a2", NotificationPriority::Now));
    assert_eq!(session.queue_len(), 2);
}

// ── 4. Background tool notification insertion ─────────────────────────────

/// Background tool notification with Later priority goes at the end.
#[test]
fn test_unified_queue_bg_notification_later_priority() {
    let mut session = ConversationSession::new("uq_bg1".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_announce("now_a", NotificationPriority::Now));
    session.push_background_tool_notification(make_bg_notification(
        "bg1",
        NotificationPriority::Later,
    ));

    let entries = session.drain_all_entries();
    assert_eq!(entries.len(), 2);
    // Now announce first, then bg notification.
    assert!(matches!(&entries[0], QueueEntry::Announce(a) if a.child_agent_id == "now_a"));
    assert!(matches!(
        &entries[1],
        QueueEntry::BackgroundToolNotification(_)
    ));
}

/// Background tool notification with Next priority goes before Later.
#[test]
fn test_unified_queue_bg_notification_next_priority() {
    let mut session = ConversationSession::new("uq_bg2".into(), "gpt-4o".into(), tmp_path());
    session.push_pending(PendingMessage::new("u1".into(), "hello".into()));
    session
        .push_background_tool_notification(make_bg_notification("bg1", NotificationPriority::Next));

    let entries = session.drain_all_entries();
    // Next bg notification before user message (Later priority).
    assert!(matches!(
        &entries[0],
        QueueEntry::BackgroundToolNotification(_)
    ));
    assert!(matches!(&entries[1], QueueEntry::UserMessage(_)));
}

// ── 5. Drain preserves FIFO within same priority and group ─────────────────

/// Two user messages (both Later): FIFO order is preserved.
#[test]
fn test_unified_queue_user_fifo_order() {
    let mut session = ConversationSession::new("uq_fifo1".into(), "gpt-4o".into(), tmp_path());
    session.push_pending(PendingMessage::new("u1".into(), "first".into()));
    session.push_pending(PendingMessage::new("u2".into(), "second".into()));
    session.push_pending(PendingMessage::new("u3".into(), "third".into()));

    let entries = session.drain_all_entries();
    let ids: Vec<&str> = entries
        .iter()
        .filter_map(|e| {
            if let QueueEntry::UserMessage(pm) = e {
                Some(pm.message_id.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(ids, vec!["u1", "u2", "u3"]);
}

/// Two Next announces: FIFO order is preserved.
#[test]
fn test_unified_queue_announce_fifo_same_priority() {
    let mut session = ConversationSession::new("uq_fifo2".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_announce("n1", NotificationPriority::Next));
    session.push_announce_to_queue(make_announce("n2", NotificationPriority::Next));

    let events = session.drain_announce_queue();
    let ids: Vec<&str> = events.iter().map(|e| e.child_agent_id.as_str()).collect();
    assert_eq!(ids, vec!["n1", "n2"]);
}

// ── 6. pop_pending skips non-user entries ──────────────────────────────────

/// pop_pending returns only user messages, re-inserting announces.
/// When a higher-priority announce is at the front, pop_pending
/// encounters it first, re-inserts it, and returns None (the user
/// message remains deeper in the queue).
#[test]
fn test_unified_queue_pop_pending_skips_announces() {
    let mut session = ConversationSession::new("uq_popp1".into(), "gpt-4o".into(), tmp_path());
    // Announce (Now) pushed first → higher priority → at front.
    session.push_announce_to_queue(make_announce("a1", NotificationPriority::Now));
    // User message (Later) pushed second → lower priority.
    session.push_pending(PendingMessage::new("u1".into(), "hello".into()));

    // pop_pending encounters the Now announce first, re-inserts it, returns None.
    assert!(session.pop_pending().is_none());
    // Both entries still in queue.
    assert_eq!(session.queue_len(), 2);
}

/// pop_pending returns user message when only user messages are
/// in the queue.
#[test]
fn test_unified_queue_pop_pending_returns_user_message() {
    let mut session = ConversationSession::new("uq_popp2".into(), "gpt-4o".into(), tmp_path());
    // Only user messages in the queue.
    session.push_pending(PendingMessage::new("u1".into(), "hello".into()));
    session.push_pending(PendingMessage::new("u2".into(), "world".into()));

    let msg = session.pop_pending().unwrap();
    assert_eq!(msg.message_id, "u1");
    assert_eq!(session.queue_len(), 1);
}

// ── 7. drain_announce_queue only returns announces ─────────────────────────

/// drain_announce_queue returns announces, re-inserting user messages.
#[test]
fn test_unified_queue_drain_announce_only() {
    let mut session = ConversationSession::new("uq_dra1".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_announce("a1", NotificationPriority::Now));
    session.push_announce_to_queue(make_announce("a2", NotificationPriority::Next));
    session.push_pending(PendingMessage::new("u1".into(), "hello".into()));

    let events = session.drain_announce_queue();
    assert_eq!(events.len(), 2);
    let ids: Vec<&str> = events.iter().map(|e| e.child_agent_id.as_str()).collect();
    assert_eq!(ids, vec!["a1", "a2"]);

    // User message should still be in the queue.
    let msgs = session.get_pending_messages();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].message_id, "u1");
}

// ── 8. Clear and clear_user_messages ───────────────────────────────────────

/// clear_queue empties everything.
#[test]
fn test_unified_queue_clear_all() {
    let mut session = ConversationSession::new("uq_clr1".into(), "gpt-4o".into(), tmp_path());
    session.push_pending(PendingMessage::new("u1".into(), "hi".into()));
    session.push_announce_to_queue(make_announce("a1", NotificationPriority::Now));
    let cleared = session.clear_queue();
    assert_eq!(cleared, 2);
    assert!(session.is_queue_empty());
}

/// clear_pending removes only user messages, preserves announces.
#[test]
fn test_unified_queue_clear_user_messages_only() {
    let mut session = ConversationSession::new("uq_clr2".into(), "gpt-4o".into(), tmp_path());
    session.push_pending(PendingMessage::new("u1".into(), "hi".into()));
    session.push_pending(PendingMessage::new("u2".into(), "hi2".into()));
    session.push_announce_to_queue(make_announce("a1", NotificationPriority::Now));

    let cleared = session.clear_pending();
    assert_eq!(cleared, 2);
    assert_eq!(session.queue_len(), 1);
    let events = session.get_announce_events();
    assert_eq!(events.len(), 1);
}

// ── 9. Comprehensive mixed scenario ────────────────────────────────────────

/// Push entries with all types and priorities; verify the full drain
/// order: Now → Next → Later, with non-user before user at each level.
#[test]
fn test_unified_queue_comprehensive_order() {
    let mut session = ConversationSession::new("uq_comp".into(), "gpt-4o".into(), tmp_path());
    session.push_pending(PendingMessage::new("u_later1".into(), "user1".into()));
    session.push_announce_to_queue(make_announce("now_a", NotificationPriority::Now));
    session.push_background_tool_notification(make_bg_notification(
        "bg_next",
        NotificationPriority::Next,
    ));
    session.push_announce_to_queue(make_announce("next_a", NotificationPriority::Next));
    session.push_pending(PendingMessage::new("u_later2".into(), "user2".into()));
    session.push_announce_to_queue(make_announce("later_a", NotificationPriority::Later));
    session.push_background_tool_notification(make_bg_notification(
        "bg_later",
        NotificationPriority::Later,
    ));

    let entries = session.drain_all_entries();
    let labels: Vec<&str> = entries
        .iter()
        .map(|e| match e {
            QueueEntry::Announce(a) => a.child_agent_id.as_str(),
            QueueEntry::UserMessage(pm) => pm.message_id.as_str(),
            QueueEntry::BackgroundToolNotification(n) => n.task_id.as_str(),
        })
        .collect();
    // Expected: Now(announce) → Next(bg+announce) → Later(announce+bg+user+user)
    assert_eq!(
        labels,
        vec!["now_a", "bg_next", "next_a", "later_a", "bg_later", "u_later1", "u_later2"]
    );
}
