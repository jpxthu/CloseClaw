//! Unit tests for InitialPermissionSet to_rules conversions (Step 1.6).
//!
//! These tests live in closeclaw-permission because the to_rules()
//! conversion logic is implemented via the InitialPermissionSetExt trait
//! defined here (respecting the dependency direction: common defines types,
//! permission provides conversion).

use closeclaw_common::permission_op::InitialPermissionSet;

use crate::engine::engine_types::{Action, Effect, Subject};
use crate::user_registry::InitialPermissionSetExt;

// ── BasicMessaging rules ────────────────────────────────────────────────────

#[test]
fn test_basic_messaging_produces_two_rules() {
    let rules = InitialPermissionSet::BasicMessaging.to_rules("ou_test");
    assert_eq!(rules.len(), 2);
}

#[test]
fn test_basic_messaging_chat_send_rule_name() {
    let rules = InitialPermissionSet::BasicMessaging.to_rules("ou_alice");
    let chat_rule = rules
        .iter()
        .find(|r| r.name.contains("chat-send"))
        .expect("chat-send rule should exist");
    assert_eq!(chat_rule.name, "user-ou_alice-chat-send");
}

#[test]
fn test_basic_messaging_workspace_read_rule_name() {
    let rules = InitialPermissionSet::BasicMessaging.to_rules("ou_alice");
    let ws_rule = rules
        .iter()
        .find(|r| r.name.contains("workspace-read"))
        .expect("workspace-read rule should exist");
    assert_eq!(ws_rule.name, "user-ou_alice-workspace-read");
}

#[test]
fn test_basic_messaging_chat_action_type() {
    let rules = InitialPermissionSet::BasicMessaging.to_rules("ou_test");
    let chat_rule = rules.iter().find(|r| r.name.contains("chat-send")).unwrap();
    match &chat_rule.actions[0] {
        Action::ToolCall { skill, methods } => {
            assert_eq!(skill, "chat");
            assert_eq!(methods, &["send"]);
        }
        other => panic!("expected ToolCall action, got {:?}", other),
    }
}

#[test]
fn test_basic_messaging_workspace_action_type() {
    let rules = InitialPermissionSet::BasicMessaging.to_rules("ou_test");
    let ws_rule = rules
        .iter()
        .find(|r| r.name.contains("workspace-read"))
        .unwrap();
    match &ws_rule.actions[0] {
        Action::File { operation, paths } => {
            assert_eq!(operation, "read");
            assert_eq!(paths, &["workspace/**"]);
        }
        other => panic!("expected File action, got {:?}", other),
    }
}

#[test]
fn test_basic_messaging_rules_all_allow() {
    let rules = InitialPermissionSet::BasicMessaging.to_rules("ou_test");
    for rule in &rules {
        assert_eq!(rule.effect, Effect::Allow);
    }
}

#[test]
fn test_basic_messaging_rules_subject_user_and_agent() {
    let rules = InitialPermissionSet::BasicMessaging.to_rules("ou_test");
    for rule in &rules {
        assert!(rule.subject.is_user_and_agent());
    }
}

#[test]
fn test_basic_messaging_rules_subject_user_id() {
    let rules = InitialPermissionSet::BasicMessaging.to_rules("ou_test");
    for rule in &rules {
        assert_eq!(rule.subject.user_id(), "ou_test");
    }
}

#[test]
fn test_basic_messaging_rules_agent_glob() {
    let rules = InitialPermissionSet::BasicMessaging.to_rules("ou_test");
    for rule in &rules {
        match &rule.subject {
            Subject::UserAndAgent {
                agent, agent_match, ..
            } => {
                assert_eq!(agent, "*");
                // MatchType::Glob displays as "Glob".
                assert!(
                    format!("{:?}", agent_match).contains("Glob"),
                    "expected Glob agent_match, got {:?}",
                    agent_match
                );
            }
            other => panic!("expected UserAndAgent subject, got {:?}", other),
        }
    }
}

#[test]
fn test_basic_messaging_rules_priority() {
    let rules = InitialPermissionSet::BasicMessaging.to_rules("ou_test");
    for rule in &rules {
        assert_eq!(rule.priority, 10);
    }
}

#[test]
fn test_basic_messaging_rules_different_user_ids() {
    let rules_a = InitialPermissionSet::BasicMessaging.to_rules("ou_a");
    let rules_b = InitialPermissionSet::BasicMessaging.to_rules("ou_b");
    // Different user IDs produce different rule names.
    assert_ne!(rules_a[0].name, rules_b[0].name);
    assert_ne!(rules_a[1].name, rules_b[1].name);
    // But same number of rules.
    assert_eq!(rules_a.len(), rules_b.len());
}
