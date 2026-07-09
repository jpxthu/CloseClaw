//! Unit tests for permission_op types (Step 1.6, migrated from permission_op.rs).

use super::*;

#[test]
fn test_describe_create_user_basic() {
    let op = PermissionOperation::CreateUser {
        user_id: "ou_123".into(),
        channel: "feishu".into(),
        initial_permissions: vec![InitialPermissionSet::BasicMessaging],
    };
    assert_eq!(
        op.describe(),
        "register user `ou_123` via feishu with permissions [BasicMessaging]"
    );
}

#[test]
fn test_describe_create_user_no_permissions() {
    let op = PermissionOperation::CreateUser {
        user_id: "ou_456".into(),
        channel: "telegram".into(),
        initial_permissions: vec![],
    };
    assert_eq!(
        op.describe(),
        "register user `ou_456` via telegram with permissions []"
    );
}

#[test]
fn test_create_user_serialization_roundtrip() {
    let op = PermissionOperation::CreateUser {
        user_id: "ou_abc".into(),
        channel: "feishu".into(),
        initial_permissions: vec![InitialPermissionSet::BasicMessaging],
    };
    let json = serde_json::to_string(&op).unwrap();
    let deserialized: PermissionOperation = serde_json::from_str(&json).unwrap();
    assert_eq!(op, deserialized);
}

#[test]
fn test_create_user_serialization_preserves_fields() {
    let op = PermissionOperation::CreateUser {
        user_id: "ou_xyz".into(),
        channel: "slack".into(),
        initial_permissions: vec![],
    };
    let json = serde_json::to_string(&op).unwrap();
    assert!(json.contains("ou_xyz"));
    assert!(json.contains("slack"));
    let deserialized: PermissionOperation = serde_json::from_str(&json).unwrap();
    match deserialized {
        PermissionOperation::CreateUser {
            user_id,
            channel,
            initial_permissions,
        } => {
            assert_eq!(user_id, "ou_xyz");
            assert_eq!(channel, "slack");
            assert!(initial_permissions.is_empty());
        }
        other => panic!("expected CreateUser, got {:?}", other),
    }
}

#[test]
fn test_initial_permission_set_serialization_roundtrip() {
    let perm = InitialPermissionSet::BasicMessaging;
    let json = serde_json::to_string(&perm).unwrap();
    let deserialized: InitialPermissionSet = serde_json::from_str(&json).unwrap();
    assert_eq!(perm, deserialized);
}

#[test]
fn test_user_registration_serialization_roundtrip() {
    let reg = UserRegistration {
        user_id: "ou_abc".into(),
        im_channel: "feishu".into(),
        initial_permissions: vec![InitialPermissionSet::BasicMessaging],
        created_at: "2026-01-01T00:00:00Z".into(),
    };
    let json = serde_json::to_string(&reg).unwrap();
    let deserialized: UserRegistration = serde_json::from_str(&json).unwrap();
    assert_eq!(reg, deserialized);
}

#[test]
fn test_user_creation_request_serialization_roundtrip() {
    let req = UserCreationRequest {
        user_id: "ou_new".into(),
        im_channel: "telegram".into(),
        request_id: "req-001".into(),
        initial_permissions: vec![InitialPermissionSet::BasicMessaging],
    };
    let json = serde_json::to_string(&req).unwrap();
    let deserialized: UserCreationRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(req, deserialized);
}

#[test]
fn test_initial_permission_set_label() {
    assert_eq!(
        InitialPermissionSet::BasicMessaging.label(),
        "BasicMessaging"
    );
}
