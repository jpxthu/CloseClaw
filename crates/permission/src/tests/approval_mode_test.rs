//!
//! ApprovalMode + RiskLevel boundary tests
//!

use crate::approval::{
    ApprovalMode, ApprovalQueue, ApproveOrDeny, RejectWhitelistReason, WhitelistTarget,
};
use crate::engine::engine_risk::RiskLevel;
use crate::engine::engine_types::{Caller, PermissionRequestBody};

fn dummy_caller() -> Caller {
    Caller {
        user_id: "test-user".to_string(),
        agent: "test-agent".to_string(),
        creator_id: "test-creator".to_string(),
    }
}

fn make_file_op(body_variant: &str) -> PermissionRequestBody {
    match body_variant {
        "a" => crate::engine::engine_types::PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/repo/a.txt".to_string(),
            op: "read".to_string(),
        },
        "b" => crate::engine::engine_types::PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/repo/b.txt".to_string(),
            op: "read".to_string(),
        },
        "c" => crate::engine::engine_types::PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/repo/c.txt".to_string(),
            op: "read".to_string(),
        },
        _ => unreachable!(),
    }
}

#[test]
fn test_approve_whitelist_low_risk() {
    let mut queue = ApprovalQueue::new();
    let id = queue
        .enqueue(
            make_file_op("a"),
            dummy_caller(),
            "low risk op".to_string(),
            RiskLevel::Low,
            "resume-1".to_string(),
            Box::new(|result| {
                assert_eq!(result, ApproveOrDeny::Approve);
            }),
        )
        .unwrap();

    let result = queue.approve(
        &id,
        ApprovalMode::WithWhitelist {
            target: WhitelistTarget::Auto,
        },
    );
    assert!(result.is_ok());
    assert!(result.unwrap());
    assert_eq!(queue.pending_count(), 0);
    assert!(queue.get_pending(&id).is_none());
}

#[test]
fn test_approve_whitelist_medium_risk() {
    let mut queue = ApprovalQueue::new();
    let id = queue
        .enqueue(
            make_file_op("a"),
            dummy_caller(),
            "medium risk op".to_string(),
            RiskLevel::Medium,
            "resume-1".to_string(),
            Box::new(|result| {
                assert_eq!(result, ApproveOrDeny::Approve);
            }),
        )
        .unwrap();

    let result = queue.approve(
        &id,
        ApprovalMode::WithWhitelist {
            target: WhitelistTarget::Auto,
        },
    );
    assert!(result.is_ok());
    assert!(result.unwrap());
    assert_eq!(queue.pending_count(), 0);
    assert!(queue.get_pending(&id).is_none());
}

#[test]
fn test_approve_whitelist_high_risk() {
    let mut queue = ApprovalQueue::new();
    let id = queue
        .enqueue(
            make_file_op("a"),
            dummy_caller(),
            "high risk op".to_string(),
            RiskLevel::High,
            "resume-1".to_string(),
            Box::new(|_| {
                panic!("callback should not be invoked on WithWhitelist reject");
            }),
        )
        .unwrap();

    let result = queue.approve(
        &id,
        ApprovalMode::WithWhitelist {
            target: WhitelistTarget::Auto,
        },
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), RejectWhitelistReason::HighRisk);
    // Request should still be pending — not resolved
    assert_eq!(queue.pending_count(), 1);
    assert!(queue.get_pending(&id).is_some());
}

#[test]
fn test_approve_whitelist_critical_risk() {
    let mut queue = ApprovalQueue::new();
    let id = queue
        .enqueue(
            make_file_op("a"),
            dummy_caller(),
            "critical risk op".to_string(),
            RiskLevel::Critical,
            "resume-1".to_string(),
            Box::new(|_| {
                panic!("callback should not be invoked on WithWhitelist reject");
            }),
        )
        .unwrap();

    let result = queue.approve(
        &id,
        ApprovalMode::WithWhitelist {
            target: WhitelistTarget::Auto,
        },
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), RejectWhitelistReason::HighRisk);
    assert_eq!(queue.pending_count(), 1);
    assert!(queue.get_pending(&id).is_some());
}

#[test]
fn test_approve_once_high_risk() {
    let mut queue = ApprovalQueue::new();
    let id = queue
        .enqueue(
            make_file_op("a"),
            dummy_caller(),
            "high risk op".to_string(),
            RiskLevel::High,
            "resume-1".to_string(),
            Box::new(|result| {
                assert_eq!(result, ApproveOrDeny::Approve);
            }),
        )
        .unwrap();

    let result = queue.approve(&id, ApprovalMode::Once);
    assert!(result.is_ok());
    assert!(result.unwrap());
    assert_eq!(queue.pending_count(), 0);
    assert!(queue.get_pending(&id).is_none());
}

#[test]
fn test_approve_once_critical_risk() {
    let mut queue = ApprovalQueue::new();
    let id = queue
        .enqueue(
            make_file_op("a"),
            dummy_caller(),
            "critical risk op".to_string(),
            RiskLevel::Critical,
            "resume-1".to_string(),
            Box::new(|result| {
                assert_eq!(result, ApproveOrDeny::Approve);
            }),
        )
        .unwrap();

    let result = queue.approve(&id, ApprovalMode::Once);
    assert!(result.is_ok());
    assert!(result.unwrap());
    assert_eq!(queue.pending_count(), 0);
    assert!(queue.get_pending(&id).is_none());
}

#[test]
fn test_approve_nonexistent_request_id() {
    let mut queue = ApprovalQueue::new();
    let result = queue.approve(
        "nonexistent-id",
        ApprovalMode::WithWhitelist {
            target: WhitelistTarget::Auto,
        },
    );
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), false);
}

#[test]
fn test_deny_behavior_unchanged() {
    let mut queue = ApprovalQueue::new();
    let body = make_file_op("a");
    let caller = dummy_caller();

    let id = queue
        .enqueue(
            body,
            caller,
            "op".to_string(),
            RiskLevel::Low,
            "resume".to_string(),
            Box::new(|result| {
                assert_eq!(result, ApproveOrDeny::Deny);
            }),
        )
        .unwrap();

    // deny does not accept ApprovalMode — behavior unchanged
    assert!(queue.deny(&id));
    assert_eq!(queue.pending_count(), 0);
    assert!(queue.get_pending(&id).is_none());

    // deny on nonexistent id still returns false
    assert!(!queue.deny("nonexistent-id"));
}
