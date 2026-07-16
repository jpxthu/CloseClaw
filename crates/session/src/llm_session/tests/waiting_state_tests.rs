use super::*;

#[test]
fn test_enter_waiting_sets_flag() {
    let session = ConversationSession::new("sess_w1".into(), "gpt-4o".into(), tmp_path());
    assert!(!session.is_waiting());
    session.enter_waiting();
    assert!(session.is_waiting());
}

#[test]
fn test_exit_waiting_clears_flag() {
    let session = ConversationSession::new("sess_w2".into(), "gpt-4o".into(), tmp_path());
    session.enter_waiting();
    assert!(session.is_waiting());
    session.exit_waiting();
    assert!(!session.is_waiting());
}

#[test]
fn test_is_waiting_default_false() {
    let session = ConversationSession::new("sess_w3".into(), "gpt-4o".into(), tmp_path());
    assert!(!session.is_waiting());
}

#[test]
fn test_enter_exit_roundtrip() {
    let session = ConversationSession::new("sess_w4".into(), "gpt-4o".into(), tmp_path());
    // enter → exit → enter → exit
    session.enter_waiting();
    assert!(session.is_waiting());
    session.exit_waiting();
    assert!(!session.is_waiting());
    session.enter_waiting();
    assert!(session.is_waiting());
    session.exit_waiting();
    assert!(!session.is_waiting());
}

#[test]
fn test_has_active_children_no_children() {
    let session = ConversationSession::new("sess_w5".into(), "gpt-4o".into(), tmp_path());
    // No children registered → no active children
    assert!(!session.has_active_children());
}

#[test]
fn test_has_active_children_with_running_child() {
    let session = ConversationSession::new("sess_w6".into(), "gpt-4o".into(), tmp_path());
    {
        let mut states = session
            .child_states
            .write()
            .expect("child_states lock poisoned");
        states.insert("child_1".into(), (ChildSessionState::Running, None));
    }
    assert!(session.has_active_children());
}

#[test]
fn test_has_active_children_all_completed() {
    let session = ConversationSession::new("sess_w7".into(), "gpt-4o".into(), tmp_path());
    {
        let mut states = session
            .child_states
            .write()
            .expect("child_states lock poisoned");
        states.insert("child_1".into(), (ChildSessionState::Completed, None));
        states.insert("child_2".into(), (ChildSessionState::Terminated, None));
    }
    assert!(!session.has_active_children());
}

#[test]
fn test_has_active_children_mixed_states() {
    let session = ConversationSession::new("sess_w8".into(), "gpt-4o".into(), tmp_path());
    {
        let mut states = session
            .child_states
            .write()
            .expect("child_states lock poisoned");
        states.insert("child_1".into(), (ChildSessionState::Completed, None));
        states.insert("child_2".into(), (ChildSessionState::Running, None));
    }
    // One still running → has active children
    assert!(session.has_active_children());
}

#[test]
fn test_has_active_children_empty_map() {
    let session = ConversationSession::new("sess_w9".into(), "gpt-4o".into(), tmp_path());
    // Explicitly empty map
    assert!(!session.has_active_children());
}

#[test]
fn test_waiting_state_debug_output() {
    let session = ConversationSession::new("sess_w10".into(), "gpt-4o".into(), tmp_path());
    session.enter_waiting();
    let debug = format!("{:?}", session);
    assert!(debug.contains("is_yielding: true"));
    session.exit_waiting();
    let debug = format!("{:?}", session);
    assert!(debug.contains("is_yielding: false"));
}
