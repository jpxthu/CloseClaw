//! Unit tests for UserRegistry (migrated from user_registry.rs).

use closeclaw_common::permission_op::InitialPermissionSet;

use crate::engine::{Action, Effect};
use crate::user_registry::{RegistryError, UserRegistry};

#[test]
fn test_new_registry_is_empty() {
    let registry = UserRegistry::new();
    assert!(registry.list_users().is_empty());
}

#[test]
fn test_register_user_returns_ruleset() {
    let mut registry = UserRegistry::new();
    let ruleset = registry
        .register_user("ou_abc", "feishu", &[InitialPermissionSet::BasicMessaging])
        .unwrap();
    assert_eq!(ruleset.rules.len(), 2);
    assert!(ruleset.rules.iter().any(|r| r.name.contains("chat-send")));
    assert!(ruleset
        .rules
        .iter()
        .any(|r| r.name.contains("workspace-read")));
}

#[test]
fn test_is_registered() {
    let mut registry = UserRegistry::new();
    assert!(!registry.is_registered("ou_abc"));
    registry
        .register_user("ou_abc", "feishu", &[InitialPermissionSet::BasicMessaging])
        .unwrap();
    assert!(registry.is_registered("ou_abc"));
    assert!(!registry.is_registered("ou_other"));
}

#[test]
fn test_duplicate_registration_errors() {
    let mut registry = UserRegistry::new();
    registry.register_user("ou_abc", "feishu", &[]).unwrap();
    let result = registry.register_user("ou_abc", "feishu", &[]);
    assert!(result.is_err());
    match result.unwrap_err() {
        RegistryError::AlreadyRegistered(id) => assert_eq!(id, "ou_abc"),
    }
}

#[test]
fn test_list_users_reflects_registrations() {
    let mut registry = UserRegistry::new();
    registry.register_user("ou_a", "feishu", &[]).unwrap();
    registry.register_user("ou_b", "telegram", &[]).unwrap();
    let users = registry.list_users();
    assert_eq!(users.len(), 2);
    assert_eq!(users[0].user_id, "ou_a");
    assert_eq!(users[1].user_id, "ou_b");
    assert_eq!(users[1].im_channel, "telegram");
}

#[test]
fn test_into_users_consumes_registry() {
    let mut registry = UserRegistry::new();
    registry.register_user("ou_x", "feishu", &[]).unwrap();
    let users = registry.into_users();
    assert_eq!(users.len(), 1);
}

#[test]
fn test_basic_messaging_rules_subject() {
    let mut registry = UserRegistry::new();
    let ruleset = registry
        .register_user("ou_test", "feishu", &[InitialPermissionSet::BasicMessaging])
        .unwrap();
    for rule in &ruleset.rules {
        assert!(rule.subject.is_user_and_agent());
        assert_eq!(rule.subject.user_id(), "ou_test");
    }
}

#[test]
fn test_basic_messaging_rules_effect() {
    let mut registry = UserRegistry::new();
    let ruleset = registry
        .register_user("ou_test", "feishu", &[InitialPermissionSet::BasicMessaging])
        .unwrap();
    for rule in &ruleset.rules {
        assert_eq!(rule.effect, Effect::Allow);
    }
}

#[test]
fn test_basic_messaging_chat_action() {
    let mut registry = UserRegistry::new();
    let ruleset = registry
        .register_user("ou_test", "feishu", &[InitialPermissionSet::BasicMessaging])
        .unwrap();
    let chat_rule = ruleset
        .rules
        .iter()
        .find(|r| r.name.contains("chat-send"))
        .expect("chat-send rule should exist");
    match &chat_rule.actions[0] {
        Action::ToolCall { skill, methods } => {
            assert_eq!(skill, "chat");
            assert_eq!(methods, &["send"]);
        }
        other => panic!("expected ToolCall action, got {:?}", other),
    }
}

#[test]
fn test_basic_messaging_workspace_action() {
    let mut registry = UserRegistry::new();
    let ruleset = registry
        .register_user("ou_test", "feishu", &[InitialPermissionSet::BasicMessaging])
        .unwrap();
    let ws_rule = ruleset
        .rules
        .iter()
        .find(|r| r.name.contains("workspace-read"))
        .expect("workspace-read rule should exist");
    match &ws_rule.actions[0] {
        Action::File { operation, paths } => {
            assert_eq!(operation, "read");
            assert_eq!(paths, &["workspace/**"]);
        }
        other => panic!("expected File action, got {:?}", other),
    }
}

#[test]
fn test_empty_permissions_produces_no_rules() {
    let mut registry = UserRegistry::new();
    let ruleset = registry.register_user("ou_test", "feishu", &[]).unwrap();
    assert!(ruleset.rules.is_empty());
    assert!(registry.is_registered("ou_test"));
}

#[test]
fn test_user_registration_metadata() {
    let mut registry = UserRegistry::new();
    registry
        .register_user("ou_meta", "feishu", &[InitialPermissionSet::BasicMessaging])
        .unwrap();
    let user = &registry.list_users()[0];
    assert_eq!(user.user_id, "ou_meta");
    assert_eq!(user.im_channel, "feishu");
    assert_eq!(
        user.initial_permissions,
        vec![InitialPermissionSet::BasicMessaging]
    );
    assert!(!user.created_at.is_empty());
}
