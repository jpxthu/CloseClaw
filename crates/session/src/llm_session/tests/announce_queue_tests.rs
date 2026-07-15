//! Tests for announce_queue priority sorting behaviour.
//!
//! Validates the `push_announce_to_queue` / `drain_announce_queue`
//! contract on [`ConversationSession`]: priority-ordered drain with
//! FIFO stability within the same priority level.

use super::*;
use chrono::Utc;
use closeclaw_common::ChildCompletionStatus;
use closeclaw_tasks::NotificationPriority;

// ── Helper ──────────────────────────────────────────────────────────────────

fn make_event(agent_id: &str, priority: NotificationPriority) -> AnnounceEvent {
    AnnounceEvent {
        child_session_id: format!("child_{}", agent_id),
        child_agent_id: agent_id.to_string(),
        result_text: format!("result from {}", agent_id),
        completed_at: Utc::now(),
        priority,
        status: ChildCompletionStatus::Completed,
    }
}

fn event_ids(events: &[AnnounceEvent]) -> Vec<&str> {
    events.iter().map(|e| e.child_agent_id.as_str()).collect()
}

// ── Normal path: mixed priorities sort correctly ─────────────────────────────

/// Push Later → Now → Next; drain must yield Now → Next → Later.
#[test]
fn test_announce_queue_priority_order() {
    let mut session = ConversationSession::new("sq_priority".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_event("later1", NotificationPriority::Later));
    session.push_announce_to_queue(make_event("now1", NotificationPriority::Now));
    session.push_announce_to_queue(make_event("next1", NotificationPriority::Next));

    let drained = session.drain_announce_queue();
    assert_eq!(event_ids(&drained), vec!["now1", "next1", "later1"]);
}

/// Push all three priorities in reverse order; drain must still sort.
#[test]
fn test_announce_queue_reverse_insertion() {
    let mut session = ConversationSession::new("sq_reverse".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_event("next1", NotificationPriority::Next));
    session.push_announce_to_queue(make_event("later1", NotificationPriority::Later));
    session.push_announce_to_queue(make_event("now1", NotificationPriority::Now));

    let drained = session.drain_announce_queue();
    assert_eq!(event_ids(&drained), vec!["now1", "next1", "later1"]);
}

// ── Same priority FIFO ───────────────────────────────────────────────────────

/// Two Next events: drain must preserve insertion order.
#[test]
fn test_announce_queue_same_priority_fifo() {
    let mut session = ConversationSession::new("sq_fifo".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_event("next_a", NotificationPriority::Next));
    session.push_announce_to_queue(make_event("next_b", NotificationPriority::Next));

    let drained = session.drain_announce_queue();
    assert_eq!(event_ids(&drained), vec!["next_a", "next_b"]);
}

/// Three Later events: FIFO order preserved.
#[test]
fn test_announce_queue_later_fifo() {
    let mut session = ConversationSession::new("sq_later_fifo".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_event("later_x", NotificationPriority::Later));
    session.push_announce_to_queue(make_event("later_y", NotificationPriority::Later));
    session.push_announce_to_queue(make_event("later_z", NotificationPriority::Later));

    let drained = session.drain_announce_queue();
    assert_eq!(event_ids(&drained), vec!["later_x", "later_y", "later_z"]);
}

/// Two Now events: FIFO order preserved.
#[test]
fn test_announce_queue_now_fifo() {
    let mut session = ConversationSession::new("sq_now_fifo".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_event("now_a", NotificationPriority::Now));
    session.push_announce_to_queue(make_event("now_b", NotificationPriority::Now));

    let drained = session.drain_announce_queue();
    assert_eq!(event_ids(&drained), vec!["now_a", "now_b"]);
}

// ── Edge cases ───────────────────────────────────────────────────────────────

/// Drain on an empty queue returns an empty Vec.
#[test]
fn test_announce_queue_empty_drain() {
    let mut session = ConversationSession::new("sq_empty".into(), "gpt-4o".into(), tmp_path());
    let drained = session.drain_announce_queue();
    assert!(drained.is_empty());
}

/// Single event: push then drain returns that event.
#[test]
fn test_announce_queue_single_event() {
    let mut session = ConversationSession::new("sq_single".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_event("only", NotificationPriority::Next));

    let drained = session.drain_announce_queue();
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].child_agent_id, "only");
}

/// Drain consumes the queue; second drain returns empty.
#[test]
fn test_announce_queue_drain_clears() {
    let mut session = ConversationSession::new("sq_clear".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_event("a", NotificationPriority::Later));
    session.push_announce_to_queue(make_event("b", NotificationPriority::Now));

    let first = session.drain_announce_queue();
    assert_eq!(first.len(), 2);

    let second = session.drain_announce_queue();
    assert!(second.is_empty());
}

// ── Full mixed scenario ──────────────────────────────────────────────────────

/// Comprehensive test: push 5 events with mixed priorities and verify
/// the final drain order matches priority ranking + FIFO within rank.
#[test]
fn test_announce_queue_full_mixed_scenario() {
    let mut session = ConversationSession::new("sq_mixed".into(), "gpt-4o".into(), tmp_path());
    session.push_announce_to_queue(make_event("L1", NotificationPriority::Later));
    session.push_announce_to_queue(make_event("N1", NotificationPriority::Now));
    session.push_announce_to_queue(make_event("X1", NotificationPriority::Next));
    session.push_announce_to_queue(make_event("L2", NotificationPriority::Later));
    session.push_announce_to_queue(make_event("N2", NotificationPriority::Now));

    let drained = session.drain_announce_queue();
    assert_eq!(
        event_ids(&drained),
        vec!["N1", "N2", "X1", "L1", "L2"],
        "Now events first, then Next, then Later; FIFO within each"
    );
}
