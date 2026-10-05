//! Unit tests for permission_op types (Step 1.6, migrated from permission_op.rs).

use super::*;

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
