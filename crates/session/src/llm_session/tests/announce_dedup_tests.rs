//! Tests for queue-level announce deduplication (Gap 2).
//!
//! Validates the dedup guard in `push_announce_to_queue`: if an event
//! with the same `child_session_id` already exists in the queue, the
//! duplicate is silently dropped.

use super::*;
use chrono::Utc;
use closeclaw_common::ChildCompletionStatus;
use closeclaw_tasks::NotificationPriority;

// ── Helpers ────────────────────────────────────────────────────────────────

fn make_event(child_id: &str, priority: NotificationPriority) -> AnnounceEvent {
    AnnounceEvent {
        child_session_id: child_id.to_string(),
        child_agent_id: format!("agent-{}", child_id),
        result_text: format!("result from {}", child_id),
        completed_at: Utc::now(),
        priority,
        status: ChildCompletionStatus::Completed,
    }
}

fn event_ids(events: &[AnnounceEvent]) -> Vec<&str> {
    events.iter().map(|e| e.child_session_id.as_str()).collect()
}

// ── Normal path: first push succeeds ───────────────────────────────────────

/// An event pushed to an empty queue should be accepted.
#[test]
fn test_dedup_first_push_succeeds() {
    let mut session = ConversationSession::new("dedup-first".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_event("child-a", NotificationPriority::Next));
    let drained = session.drain_announce_queue();
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].child_session_id, "child-a");
}

// ── Dedup path: same child_session_id is skipped ───────────────────────────

/// Pushing two events with the same `child_session_id` must result in
/// only the first event remaining in the queue.
#[test]
fn test_dedup_same_child_id_skipped() {
    let mut session = ConversationSession::new("dedup-skip".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_event("child-x", NotificationPriority::Next));
    session.push_announce_to_queue(make_event("child-x", NotificationPriority::Now));

    let drained = session.drain_announce_queue();
    assert_eq!(drained.len(), 1, "duplicate should be dropped");
    assert_eq!(drained[0].child_session_id, "child-x");
}

// ── Different children enter independently ──────────────────────────────────

/// Events with distinct `child_session_id` values must all be accepted.
#[test]
fn test_dedup_different_children_independent() {
    let mut session = ConversationSession::new("dedup-multi".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_event("child-1", NotificationPriority::Next));
    session.push_announce_to_queue(make_event("child-2", NotificationPriority::Later));
    session.push_announce_to_queue(make_event("child-3", NotificationPriority::Now));

    let drained = session.drain_announce_queue();
    assert_eq!(
        drained.len(),
        3,
        "all three unique children should be present"
    );
    assert_eq!(
        event_ids(&drained),
        vec!["child-3", "child-1", "child-2"],
        "Now first, then Next, then Later"
    );
}

// ── Duplicate among multiple different children ─────────────────────────────

/// If a duplicate appears after several unique events, only the
/// duplicate is dropped — the others remain.
#[test]
fn test_dedup_duplicate_among_multiple() {
    let mut session = ConversationSession::new("dedup-mixed".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_event("child-1", NotificationPriority::Later));
    session.push_announce_to_queue(make_event("child-2", NotificationPriority::Now));
    session.push_announce_to_queue(make_event("child-3", NotificationPriority::Next));

    // Duplicate of child-2 should be skipped.
    session.push_announce_to_queue(make_event("child-2", NotificationPriority::Later));

    let drained = session.drain_announce_queue();
    assert_eq!(drained.len(), 3, "only the duplicate should be dropped");
    assert_eq!(event_ids(&drained), vec!["child-2", "child-3", "child-1"]);
}

// ── Priority ordering preserved after dedup ─────────────────────────────────

/// Dedup must not disturb priority ordering. Push Now + Next, then
/// duplicate the Now event; the queue should still have Now → Next.
#[test]
fn test_dedup_preserves_priority_order() {
    let mut session = ConversationSession::new("dedup-order".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_event("now-child", NotificationPriority::Now));
    session.push_announce_to_queue(make_event("next-child", NotificationPriority::Next));

    // Duplicate: should be skipped.
    session.push_announce_to_queue(make_event("now-child", NotificationPriority::Now));

    let drained = session.drain_announce_queue();
    assert_eq!(
        event_ids(&drained),
        vec!["now-child", "next-child"],
        "Now must still come before Next"
    );
}

// ── Multiple duplicates of the same child ──────────────────────────────────

/// Pushing the same `child_session_id` three times should yield
/// exactly one event in the queue.
#[test]
fn test_dedup_triple_duplicate() {
    let mut session = ConversationSession::new("dedup-triple".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_event("child-a", NotificationPriority::Next));
    session.push_announce_to_queue(make_event("child-a", NotificationPriority::Now));
    session.push_announce_to_queue(make_event("child-a", NotificationPriority::Later));

    let drained = session.drain_announce_queue();
    assert_eq!(drained.len(), 1, "three duplicates should reduce to one");
    assert_eq!(drained[0].child_session_id, "child-a");
}

// ── Empty string child_session_id deduplicates ──────────────────────────────

/// Events with an empty `child_session_id` should be deduplicated
/// against each other (since the field is `String`, not `Option`).
#[test]
fn test_dedup_empty_string_child_id() {
    let mut session = ConversationSession::new("dedup-empty".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_event("", NotificationPriority::Next));
    session.push_announce_to_queue(make_event("", NotificationPriority::Now));

    let drained = session.drain_announce_queue();
    assert_eq!(
        drained.len(),
        1,
        "empty-string child IDs should be deduplicated"
    );
}
