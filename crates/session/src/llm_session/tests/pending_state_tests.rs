use super::*;
use crate::persistence::PendingMessage;

// ── pending_messages queue ──────────────────────────────────────────────────

#[test]
fn test_pending_initial_state() {
    let session = ConversationSession::new("sess_pending".into(), "gpt-4o".into(), tmp_path());
    assert_eq!(session.pending_count(), 0);
    assert!(!session.has_pending());
}

#[test]
fn test_push_pending_sets_has_pending_and_increments_count() {
    let mut session = ConversationSession::new("sess_pending".into(), "gpt-4o".into(), tmp_path());
    assert_eq!(session.pending_count(), 0);
    session.push_pending(PendingMessage::new("msg_1".into(), "hello".into()));
    assert!(session.has_pending());
    assert_eq!(session.pending_count(), 1);
    session.push_pending(PendingMessage::new("msg_2".into(), "world".into()));
    assert_eq!(session.pending_count(), 2);
}

#[test]
fn test_pop_pending_fifo_order() {
    let mut session = ConversationSession::new("sess_fifo".into(), "gpt-4o".into(), tmp_path());
    session.push_pending(PendingMessage::new("msg_A".into(), "first".into()));
    session.push_pending(PendingMessage::new("msg_B".into(), "second".into()));
    let first = session.pop_pending();
    assert!(first.is_some());
    assert_eq!(first.unwrap().message_id, "msg_A");
    let second = session.pop_pending();
    assert!(second.is_some());
    assert_eq!(second.unwrap().message_id, "msg_B");
}

#[test]
fn test_pop_pending_returns_none_when_empty() {
    let mut session = ConversationSession::new("sess_empty".into(), "gpt-4o".into(), tmp_path());
    assert!(session.pop_pending().is_none());
}

#[test]
fn test_get_pending_messages_does_not_consume_queue() {
    let mut session = ConversationSession::new("sess_get".into(), "gpt-4o".into(), tmp_path());
    session.push_pending(PendingMessage::new("msg_1".into(), "hello".into()));
    session.push_pending(PendingMessage::new("msg_2".into(), "world".into()));

    let msgs = session.get_pending_messages();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].message_id, "msg_1");
    assert_eq!(msgs[1].message_id, "msg_2");

    assert_eq!(session.pending_count(), 2);

    let popped = session.pop_pending().unwrap();
    assert_eq!(popped.message_id, "msg_1");
    assert_eq!(session.pending_count(), 1);
}

#[test]
fn test_get_pending_messages_empty() {
    let session = ConversationSession::new("sess_empty_get".into(), "gpt-4o".into(), tmp_path());
    let msgs = session.get_pending_messages();
    assert!(msgs.is_empty());
}

#[test]
fn test_restore_pending_messages_only_sent_false() {
    let mut session = ConversationSession::new("sess_restore".into(), "gpt-4o".into(), tmp_path());

    let mut sent_msg = PendingMessage::new("sent_1".into(), "already sent".into());
    sent_msg.mark_sent();
    let unsent_msg = PendingMessage::new("unsent_1".into(), "not sent yet".into());

    session.restore_pending_messages(vec![sent_msg, unsent_msg]);

    assert_eq!(session.pending_count(), 1);
    let restored = session.pop_pending().unwrap();
    assert_eq!(restored.message_id, "unsent_1");
    assert!(!restored.sent);
}

#[test]
fn test_restore_pending_messages_skips_all_sent() {
    let mut session =
        ConversationSession::new("sess_restore_skip".into(), "gpt-4o".into(), tmp_path());

    let mut msg1 = PendingMessage::new("a".into(), "a".into());
    msg1.mark_sent();
    let mut msg2 = PendingMessage::new("b".into(), "b".into());
    msg2.mark_sent();

    session.restore_pending_messages(vec![msg1, msg2]);
    assert_eq!(session.pending_count(), 0);
}

#[test]
fn test_restore_pending_messages_empty_vec_no_op() {
    let mut session =
        ConversationSession::new("sess_restore_empty".into(), "gpt-4o".into(), tmp_path());
    session.push_pending(PendingMessage::new("existing".into(), "existing".into()));

    session.restore_pending_messages(vec![]);
    assert_eq!(session.pending_count(), 1);
    let popped = session.pop_pending().unwrap();
    assert_eq!(popped.message_id, "existing");
}

// ── clear_pending tests ─────────────────────────────────────────────────

#[test]
fn test_clear_pending_returns_count_and_empties() {
    let mut session = ConversationSession::new("sess_clear".into(), "gpt-4o".into(), tmp_path());
    session.push_pending(PendingMessage::new("a".into(), "msg_a".into()));
    session.push_pending(PendingMessage::new("b".into(), "msg_b".into()));
    assert_eq!(session.pending_count(), 2);

    let cleared = session.clear_pending();
    assert_eq!(cleared, 2);
    assert_eq!(session.pending_count(), 0);
    assert!(session.get_pending_messages().is_empty());
}

#[test]
fn test_clear_pending_empty_returns_zero() {
    let mut session =
        ConversationSession::new("sess_clear_empty".into(), "gpt-4o".into(), tmp_path());
    let cleared = session.clear_pending();
    assert_eq!(cleared, 0);
}
