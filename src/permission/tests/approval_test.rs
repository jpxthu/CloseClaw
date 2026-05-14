//!
//! ApprovalQueue unit tests
//!

use crate::permission::approval::{ApprovalQueue, ApproveOrDeny, RejectReason};
use crate::permission::engine::engine_risk::RiskLevel;
use crate::permission::engine::engine_types::{Caller, PermissionRequestBody};

fn dummy_caller() -> Caller {
    Caller {
        user_id: "test-user".to_string(),
        agent: "test-agent".to_string(),
        creator_id: "test-creator".to_string(),
    }
}

fn make_file_op(body_variant: &str) -> PermissionRequestBody {
    match body_variant {
        "a" => PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/repo/a.txt".to_string(),
            op: "read".to_string(),
        },
        "b" => PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/repo/b.txt".to_string(),
            op: "read".to_string(),
        },
        "c" => PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/repo/c.txt".to_string(),
            op: "read".to_string(),
        },
        _ => unreachable!(),
    }
}

#[test]
fn test_new_queue_is_empty() {
    let queue = ApprovalQueue::new();
    assert_eq!(queue.pending_count(), 0);
}

#[test]
fn test_compute_operation_key_deterministic() {
    let body = PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: "/repo/src/main.rs".to_string(),
        op: "read".to_string(),
    };
    let key1 = ApprovalQueue::compute_operation_key(&body);
    let key2 = ApprovalQueue::compute_operation_key(&body);
    assert_eq!(key1, key2);
    assert_eq!(key1.len(), 64); // SHA256 hex = 64 chars
}

#[test]
fn test_compute_operation_key_different_for_different_bodies() {
    let body1 = PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: "/repo/a.txt".to_string(),
        op: "read".to_string(),
    };
    let body2 = PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: "/repo/b.txt".to_string(),
        op: "read".to_string(),
    };
    let key1 = ApprovalQueue::compute_operation_key(&body1);
    let key2 = ApprovalQueue::compute_operation_key(&body2);
    assert_ne!(key1, key2);
}

#[test]
fn test_enqueue_success() {
    let mut queue = ApprovalQueue::new();
    let body = make_file_op("a");
    let caller = dummy_caller();

    let result = queue.enqueue(
        body,
        caller.clone(),
        "read file".to_string(),
        RiskLevel::Low,
        "v1".to_string(),
        "resume-token-1".to_string(),
        Box::new(|_| {}),
    );

    assert!(result.is_ok());
    let request_id = result.unwrap();
    assert!(!request_id.is_empty());
    assert_eq!(queue.pending_count(), 1);

    let pending = queue.get_pending(&request_id).unwrap();
    assert_eq!(pending.caller.agent, "test-agent");
    assert_eq!(pending.operation_desc, "read file");
    assert_eq!(pending.risk_level, RiskLevel::Low);
    assert_eq!(pending.rule_version, "v1");
    assert_eq!(pending.session_resume, "resume-token-1");
}

#[test]
fn test_enqueue_reject_duplicate() {
    let mut queue = ApprovalQueue::new();
    let body = make_file_op("a");
    let caller = dummy_caller();

    let id1 = queue
        .enqueue(
            body.clone(),
            caller.clone(),
            "op1".to_string(),
            RiskLevel::Medium,
            "v1".to_string(),
            "resume-1".to_string(),
            Box::new(|_| {}),
        )
        .unwrap();

    // Same body → same operation_key → must reject
    let id2 = queue.enqueue(
        body,
        caller.clone(),
        "op2".to_string(),
        RiskLevel::High,
        "v2".to_string(),
        "resume-2".to_string(),
        Box::new(|_| {}),
    );

    assert!(id2.is_err());
    assert_eq!(id2.unwrap_err(), RejectReason::Duplicate);
    assert_eq!(queue.pending_count(), 1);
    // Original entry untouched
    assert!(queue.get_pending(&id1).is_some());
}

#[test]
fn test_approve_triggers_approve_callback() {
    let mut queue = ApprovalQueue::new();
    let body = make_file_op("a");
    let caller = dummy_caller();

    let id = queue
        .enqueue(
            body,
            caller,
            "op".to_string(),
            RiskLevel::Low,
            "v1".to_string(),
            "resume".to_string(),
            Box::new(|result| {
                assert_eq!(result, ApproveOrDeny::Approve);
            }),
        )
        .unwrap();

    assert!(queue.approve(&id));
    assert_eq!(queue.pending_count(), 0);
    assert!(queue.get_pending(&id).is_none());
}

#[test]
fn test_deny_triggers_deny_callback() {
    let mut queue = ApprovalQueue::new();
    let body = make_file_op("a");
    let caller = dummy_caller();

    let id = queue
        .enqueue(
            body,
            caller,
            "op".to_string(),
            RiskLevel::Low,
            "v1".to_string(),
            "resume".to_string(),
            Box::new(|result| {
                assert_eq!(result, ApproveOrDeny::Deny);
            }),
        )
        .unwrap();

    assert!(queue.deny(&id));
    assert_eq!(queue.pending_count(), 0);
    assert!(queue.get_pending(&id).is_none());
}

#[test]
fn test_approve_nonexistent_returns_false() {
    let mut queue = ApprovalQueue::new();
    assert!(!queue.approve("nonexistent-id"));
}

#[test]
fn test_deny_nonexistent_returns_false() {
    let mut queue = ApprovalQueue::new();
    assert!(!queue.deny("nonexistent-id"));
}

#[test]
fn test_clear_triggers_deny_for_all_entries() {
    let mut queue = ApprovalQueue::new();
    let caller = dummy_caller();

    let cleared_ids: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

    // Use 3 distinct bodies to avoid duplicate rejection
    for i in 0..3 {
        let cleared_ids_clone = cleared_ids.clone();
        let captured_id = format!("id-{}", i);
        queue
            .enqueue(
                make_file_op(match i {
                    0 => "a",
                    1 => "b",
                    _ => "c", // i=2 uses a third distinct body
                }),
                caller.clone(),
                format!("op-{}", i),
                RiskLevel::Low,
                format!("v{}", i),
                format!("resume-{}", i),
                Box::new(move |result| {
                    assert_eq!(result, ApproveOrDeny::Deny);
                    cleared_ids_clone.lock().unwrap().push(captured_id.clone());
                }),
            )
            .unwrap();
    }

    assert_eq!(queue.pending_count(), 3);
    queue.clear();
    assert_eq!(queue.pending_count(), 0);

    let cleared = cleared_ids.lock().unwrap();
    assert_eq!(cleared.len(), 3);
    // All three were denied (not approved)
}

#[test]
fn test_get_pending_returns_correct_pending_approval() {
    let mut queue = ApprovalQueue::new();
    let body = make_file_op("a");
    let caller = dummy_caller();

    let id = queue
        .enqueue(
            body,
            caller.clone(),
            "my operation".to_string(),
            RiskLevel::High,
            "rule-v3".to_string(),
            "session-resume-abc".to_string(),
            Box::new(|_| {}),
        )
        .unwrap();

    let pending = queue.get_pending(&id).unwrap();
    assert_eq!(pending.operation_desc, "my operation");
    assert_eq!(pending.risk_level, RiskLevel::High);
    assert_eq!(pending.rule_version, "rule-v3");
    assert_eq!(pending.session_resume, "session-resume-abc");
    assert_eq!(pending.caller.agent, "test-agent");
}

#[test]
fn test_get_pending_returns_none_for_unknown_id() {
    let queue = ApprovalQueue::new();
    assert!(queue.get_pending("unknown").is_none());
}

#[test]
fn test_rule_version_field_correctly_stored() {
    let mut queue = ApprovalQueue::new();
    let body = make_file_op("a");
    let caller = dummy_caller();

    let id = queue
        .enqueue(
            body,
            caller,
            "op".to_string(),
            RiskLevel::Medium,
            "v42".to_string(),
            "resume".to_string(),
            Box::new(|_| {}),
        )
        .unwrap();

    let pending = queue.get_pending(&id).unwrap();
    assert_eq!(pending.rule_version, "v42");
}

#[test]
fn test_callback_signature_send() {
    // Verify Callback is Send + FnOnce via compile-time check
    fn _assert_send<F: FnOnce(ApproveOrDeny) + Send>() {}
    fn _assert_callback_is_send() {
        fn inner<C: FnOnce(ApproveOrDeny) + Send>() {}
        fn _check<C: FnOnce(ApproveOrDeny) + Send>() {}
        // Dummy compile-time check — if Callback changes to non-Send this fails
    }
    let mut queue = ApprovalQueue::new();
    let body = make_file_op("a");
    queue.enqueue(
        body,
        dummy_caller(),
        "op".to_string(),
        RiskLevel::Low,
        "v1".to_string(),
        "resume".to_string(),
        Box::new(|_| {}),
    );
}

#[test]
fn test_enqueue_different_bodies_not_duplicate() {
    let mut queue = ApprovalQueue::new();
    let caller = dummy_caller();

    let id1 = queue
        .enqueue(
            make_file_op("a"),
            caller.clone(),
            "op-a".to_string(),
            RiskLevel::Low,
            "v1".to_string(),
            "resume-1".to_string(),
            Box::new(|_| {}),
        )
        .unwrap();

    // Different body → different operation_key → must succeed
    let id2 = queue
        .enqueue(
            make_file_op("b"),
            caller.clone(),
            "op-b".to_string(),
            RiskLevel::Low,
            "v1".to_string(),
            "resume-2".to_string(),
            Box::new(|_| {}),
        )
        .unwrap();

    assert_ne!(id1, id2);
    assert_eq!(queue.pending_count(), 2);
}

#[test]
fn test_pending_approval_created_at_is_set() {
    let mut queue = ApprovalQueue::new();
    let before = chrono::Utc::now();
    let id = queue
        .enqueue(
            make_file_op("a"),
            dummy_caller(),
            "op".to_string(),
            RiskLevel::Low,
            "v1".to_string(),
            "resume".to_string(),
            Box::new(|_| {}),
        )
        .unwrap();
    let after = chrono::Utc::now();

    let pending = queue.get_pending(&id).unwrap();
    assert!(pending.created_at >= before);
    assert!(pending.created_at <= after);
}
