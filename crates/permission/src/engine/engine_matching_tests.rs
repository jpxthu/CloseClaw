use super::{action_matches_request, Action, MessageDirection, PermissionRequestBody};
use crate::actions::ActionBuilder;

// ── helpers ──────────────────────────────────────────────────────────────

fn msg_send(agent: &str, direction: MessageDirection, target: &str) -> PermissionRequestBody {
    PermissionRequestBody::MessageSend {
        agent: agent.to_string(),
        direction,
        target: target.to_string(),
    }
}

// ── 1. normal path ───────────────────────────────────────────────────────

#[test]
fn test_message_both_matches_any_send() {
    let action = Action::Message {
        direction: MessageDirection::Both,
        targets: vec![],
    };
    let req = msg_send("a", MessageDirection::Send, "chat_1");
    assert!(action_matches_request(&action, &req));
}

#[test]
fn test_message_both_matches_any_receive() {
    let action = Action::Message {
        direction: MessageDirection::Both,
        targets: vec![],
    };
    let req = msg_send("a", MessageDirection::Receive, "chat_1");
    assert!(action_matches_request(&action, &req));
}

// ── 2. direction filtering ───────────────────────────────────────────────

#[test]
fn test_message_send_only_matches_send_request() {
    let action = Action::Message {
        direction: MessageDirection::Send,
        targets: vec![],
    };
    let req = msg_send("a", MessageDirection::Send, "chat_1");
    assert!(action_matches_request(&action, &req));
}

#[test]
fn test_message_send_does_not_match_receive_request() {
    let action = Action::Message {
        direction: MessageDirection::Send,
        targets: vec![],
    };
    let req = msg_send("a", MessageDirection::Receive, "chat_1");
    assert!(!action_matches_request(&action, &req));
}

#[test]
fn test_message_receive_only_matches_receive_request() {
    let action = Action::Message {
        direction: MessageDirection::Receive,
        targets: vec![],
    };
    let req = msg_send("a", MessageDirection::Receive, "chat_1");
    assert!(action_matches_request(&action, &req));
}

#[test]
fn test_message_receive_does_not_match_send_request() {
    let action = Action::Message {
        direction: MessageDirection::Receive,
        targets: vec![],
    };
    let req = msg_send("a", MessageDirection::Send, "chat_1");
    assert!(!action_matches_request(&action, &req));
}

// ── 3. target filtering ──────────────────────────────────────────────────

#[test]
fn test_message_target_glob_matches() {
    let action = Action::Message {
        direction: MessageDirection::Both,
        targets: vec!["chat_*".to_string()],
    };
    let req = msg_send("a", MessageDirection::Send, "chat_123");
    assert!(action_matches_request(&action, &req));
}

#[test]
fn test_message_target_glob_no_match() {
    let action = Action::Message {
        direction: MessageDirection::Both,
        targets: vec!["chat_*".to_string()],
    };
    let req = msg_send("a", MessageDirection::Send, "other");
    assert!(!action_matches_request(&action, &req));
}

// ── 4. both direction compat ─────────────────────────────────────────────

#[test]
fn test_message_both_matches_send_direction() {
    let action = Action::Message {
        direction: MessageDirection::Both,
        targets: vec![],
    };
    let req = msg_send("a", MessageDirection::Send, "inbox");
    assert!(action_matches_request(&action, &req));
}

#[test]
fn test_message_both_matches_receive_direction() {
    let action = Action::Message {
        direction: MessageDirection::Both,
        targets: vec![],
    };
    let req = msg_send("a", MessageDirection::Receive, "inbox");
    assert!(action_matches_request(&action, &req));
}

// ── 5. builder test ──────────────────────────────────────────────────────

#[test]
fn test_message_builder_basic() {
    let action = ActionBuilder::message(MessageDirection::Send)
        .build()
        .unwrap();

    match action {
        Action::Message { direction, targets } => {
            assert_eq!(direction, MessageDirection::Send);
            assert!(targets.is_empty());
        }
        _ => panic!("expected Message action"),
    }
}

#[test]
fn test_message_builder_with_targets() {
    let action = ActionBuilder::message(MessageDirection::Receive)
        .with_targets(vec!["chat_*".to_string(), "dm_*".to_string()])
        .build()
        .unwrap();

    match action {
        Action::Message { direction, targets } => {
            assert_eq!(direction, MessageDirection::Receive);
            assert_eq!(targets, vec!["chat_*".to_string(), "dm_*".to_string()]);
        }
        _ => panic!("expected Message action"),
    }
}

// ── 6. serde round-trip ──────────────────────────────────────────────────

#[test]
fn test_message_action_serde_round_trip() {
    let action = Action::Message {
        direction: MessageDirection::Send,
        targets: vec!["chat_*".to_string()],
    };
    let json = serde_json::to_string(&action).unwrap();
    let deserialized: Action = serde_json::from_str(&json).unwrap();

    match deserialized {
        Action::Message { direction, targets } => {
            assert_eq!(direction, MessageDirection::Send);
            assert_eq!(targets, vec!["chat_*".to_string()]);
        }
        _ => panic!("expected Message action"),
    }
}

#[test]
fn test_message_action_serde_tag() {
    let json = r#"{"type":"message","direction":"receive","targets":["dm_*"]}"#;
    let action: Action = serde_json::from_str(json).unwrap();

    match action {
        Action::Message { direction, targets } => {
            assert_eq!(direction, MessageDirection::Receive);
            assert_eq!(targets, vec!["dm_*".to_string()]);
        }
        _ => panic!("expected Message action"),
    }
}

#[test]
fn test_message_direction_serde_defaults() {
    let json = r#"{"type":"message","direction":"both"}"#;
    let action: Action = serde_json::from_str(json).unwrap();

    match action {
        Action::Message { direction, targets } => {
            assert_eq!(direction, MessageDirection::Both);
            assert!(targets.is_empty());
        }
        _ => panic!("expected Message action"),
    }
}

// ── 7. request body serde ────────────────────────────────────────────────

#[test]
fn test_message_send_request_serde_round_trip() {
    let body = PermissionRequestBody::MessageSend {
        agent: "agent1".to_string(),
        direction: MessageDirection::Send,
        target: "chat_42".to_string(),
    };
    let json = serde_json::to_string(&body).unwrap();
    let deserialized: PermissionRequestBody = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.dimension_name(), Some("message"));
    assert_eq!(deserialized.agent_id(), "agent1");
}
