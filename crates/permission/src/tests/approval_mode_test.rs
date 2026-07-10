//!
//! ApprovalMode + RiskLevel boundary tests
//!

use crate::approval::{
    ApprovalMode, ApprovalQueue, ApproveOrDeny, EnqueueRequest, RejectWhitelistReason,
    WhitelistTarget,
};
use crate::engine::engine_risk::RiskLevel;
use crate::engine::engine_types::{Caller, PermissionRequestBody, RuleSet};

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

fn default_rules() -> RuleSet {
    RuleSet::default()
}

#[test]
fn test_approve_whitelist_low_risk() {
    let mut queue = ApprovalQueue::new();
    let id = queue
        .enqueue(
            EnqueueRequest {
                request: make_file_op("a"),
                caller: dummy_caller(),
                operation_desc: "low risk op".to_string(),
                risk_level: RiskLevel::Low,
                session_resume: "resume-1".to_string(),
                callback: Box::new(|result| {
                    assert_eq!(result, ApproveOrDeny::Approve);
                }),
            },
            &default_rules(),
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
            EnqueueRequest {
                request: make_file_op("a"),
                caller: dummy_caller(),
                operation_desc: "medium risk op".to_string(),
                risk_level: RiskLevel::Medium,
                session_resume: "resume-1".to_string(),
                callback: Box::new(|result| {
                    assert_eq!(result, ApproveOrDeny::Approve);
                }),
            },
            &default_rules(),
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
            EnqueueRequest {
                request: make_file_op("a"),
                caller: dummy_caller(),
                operation_desc: "high risk op".to_string(),
                risk_level: RiskLevel::High,
                session_resume: "resume-1".to_string(),
                callback: Box::new(|_| {
                    panic!("callback should not be invoked on WithWhitelist reject");
                }),
            },
            &default_rules(),
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
            EnqueueRequest {
                request: make_file_op("a"),
                caller: dummy_caller(),
                operation_desc: "critical risk op".to_string(),
                risk_level: RiskLevel::Critical,
                session_resume: "resume-1".to_string(),
                callback: Box::new(|_| {
                    panic!("callback should not be invoked on WithWhitelist reject");
                }),
            },
            &default_rules(),
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
            EnqueueRequest {
                request: make_file_op("a"),
                caller: dummy_caller(),
                operation_desc: "high risk op".to_string(),
                risk_level: RiskLevel::High,
                session_resume: "resume-1".to_string(),
                callback: Box::new(|result| {
                    assert_eq!(result, ApproveOrDeny::Approve);
                }),
            },
            &default_rules(),
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
            EnqueueRequest {
                request: make_file_op("a"),
                caller: dummy_caller(),
                operation_desc: "critical risk op".to_string(),
                risk_level: RiskLevel::Critical,
                session_resume: "resume-1".to_string(),
                callback: Box::new(|result| {
                    assert_eq!(result, ApproveOrDeny::Approve);
                }),
            },
            &default_rules(),
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
            EnqueueRequest {
                request: body,
                caller: caller,
                operation_desc: "op".to_string(),
                risk_level: RiskLevel::Low,
                session_resume: "resume".to_string(),
                callback: Box::new(|result| {
                    assert_eq!(result, ApproveOrDeny::Deny);
                }),
            },
            &default_rules(),
        )
        .unwrap();

    // deny does not accept ApprovalMode — behavior unchanged
    assert!(queue.deny(&id));
    assert_eq!(queue.pending_count(), 0);
    assert!(queue.get_pending(&id).is_none());

    // deny on nonexistent id still returns false
    assert!(!queue.deny("nonexistent-id"));
}

// ===========================================================================
// WhitelistTarget 枚举 + caller_to_subject 行为测试
// ===========================================================================

use crate::whitelist::caller_to_subject;

fn owner_caller() -> Caller {
    Caller {
        user_id: "owner".to_string(),
        agent: "test-agent".to_string(),
        creator_id: String::new(),
    }
}

fn non_owner_caller() -> Caller {
    Caller {
        user_id: "user-42".to_string(),
        agent: "test-agent".to_string(),
        creator_id: "creator-99".to_string(),
    }
}

fn empty_user_caller() -> Caller {
    Caller {
        user_id: String::new(),
        agent: "test-agent".to_string(),
        creator_id: String::new(),
    }
}

// -- WhitelistTarget::Auto --------------------------------------------------

#[test]
fn test_auto_owner_caller_produces_agent_only() {
    let subject = caller_to_subject(&owner_caller(), WhitelistTarget::Auto);
    assert!(subject.is_agent_only(), "Owner → AgentOnly");
    assert_eq!(subject.agent_id(), "test-agent");
}

#[test]
fn test_auto_non_owner_caller_produces_user_and_agent() {
    let subject = caller_to_subject(&non_owner_caller(), WhitelistTarget::Auto);
    assert!(subject.is_user_and_agent(), "Non-owner → UserAndAgent");
    assert_eq!(subject.agent_id(), "test-agent");
    assert_eq!(subject.user_id(), "user-42");
}

#[test]
fn test_auto_empty_user_id_produces_agent_only() {
    let subject = caller_to_subject(&empty_user_caller(), WhitelistTarget::Auto);
    assert!(subject.is_agent_only(), "Empty user_id → AgentOnly");
}

// -- WhitelistTarget::AgentOnly --------------------------------------------

#[test]
fn test_agent_only_owner_produces_agent_only() {
    let subject = caller_to_subject(&owner_caller(), WhitelistTarget::AgentOnly);
    assert!(
        subject.is_agent_only(),
        "AgentOnly target → always AgentOnly"
    );
    assert_eq!(subject.agent_id(), "test-agent");
}

#[test]
fn test_agent_only_non_owner_produces_agent_only() {
    let subject = caller_to_subject(&non_owner_caller(), WhitelistTarget::AgentOnly);
    assert!(
        subject.is_agent_only(),
        "AgentOnly target ignores caller identity"
    );
}

#[test]
fn test_agent_only_empty_user_produces_agent_only() {
    let subject = caller_to_subject(&empty_user_caller(), WhitelistTarget::AgentOnly);
    assert!(subject.is_agent_only());
}

// -- WhitelistTarget::UserAndAgent -----------------------------------------

#[test]
fn test_user_and_agent_with_user_id_produces_user_and_agent() {
    let subject = caller_to_subject(&non_owner_caller(), WhitelistTarget::UserAndAgent);
    assert!(
        subject.is_user_and_agent(),
        "UserAndAgent + user_id → UserAndAgent"
    );
    assert_eq!(subject.user_id(), "user-42");
    assert_eq!(subject.agent_id(), "test-agent");
}

#[test]
fn test_user_and_agent_empty_user_id_falls_back_to_agent_only() {
    let subject = caller_to_subject(&empty_user_caller(), WhitelistTarget::UserAndAgent);
    assert!(
        subject.is_agent_only(),
        "UserAndAgent + empty user_id → fallback AgentOnly"
    );
}

#[test]
fn test_user_and_agent_owner_with_user_id_produces_user_and_agent() {
    // Even owner caller, when target is UserAndAgent and user_id is present,
    // should produce UserAndAgent (target overrides Auto inference).
    let caller = Caller {
        user_id: "owner".to_string(),
        agent: "test-agent".to_string(),
        creator_id: String::new(),
    };
    let subject = caller_to_subject(&caller, WhitelistTarget::UserAndAgent);
    assert!(
        subject.is_user_and_agent(),
        "UserAndAgent target + non-empty user_id"
    );
    assert_eq!(subject.user_id(), "owner");
}

// -- approve() with new WithWhitelist signature ----------------------------

#[test]
fn test_approve_with_agent_only_target_low_risk() {
    let mut queue = ApprovalQueue::new();
    let id = queue
        .enqueue(
            EnqueueRequest {
                request: make_file_op("a"),
                caller: dummy_caller(),
                operation_desc: "low risk op".to_string(),
                risk_level: RiskLevel::Low,
                session_resume: "resume-1".to_string(),
                callback: Box::new(|result| {
                    assert_eq!(result, ApproveOrDeny::Approve);
                }),
            },
            &default_rules(),
        )
        .unwrap();

    let result = queue.approve(
        &id,
        ApprovalMode::WithWhitelist {
            target: WhitelistTarget::AgentOnly,
        },
    );
    assert!(result.is_ok());
    assert!(result.unwrap());
    assert_eq!(queue.pending_count(), 0);
}

#[test]
fn test_approve_with_user_and_agent_target_medium_risk() {
    let mut queue = ApprovalQueue::new();
    let id = queue
        .enqueue(
            EnqueueRequest {
                request: make_file_op("a"),
                caller: dummy_caller(),
                operation_desc: "medium risk op".to_string(),
                risk_level: RiskLevel::Medium,
                session_resume: "resume-1".to_string(),
                callback: Box::new(|result| {
                    assert_eq!(result, ApproveOrDeny::Approve);
                }),
            },
            &default_rules(),
        )
        .unwrap();

    let result = queue.approve(
        &id,
        ApprovalMode::WithWhitelist {
            target: WhitelistTarget::UserAndAgent,
        },
    );
    assert!(result.is_ok());
    assert!(result.unwrap());
    assert_eq!(queue.pending_count(), 0);
}

// ===========================================================================
// ConfigWrite hard-coded whitelist rejection tests
// ===========================================================================

#[test]
fn test_config_write_with_whitelist_rejected_regardless_of_risk_level() {
    for risk in [
        RiskLevel::Low,
        RiskLevel::Medium,
        RiskLevel::High,
        RiskLevel::Critical,
    ] {
        let mut queue = ApprovalQueue::new();
        let id = queue
            .enqueue(
                EnqueueRequest {
                    request: PermissionRequestBody::ConfigWrite {
                        agent: "test-agent".to_string(),
                        config_file: "/etc/app.toml".to_string(),
                    },
                    caller: dummy_caller(),
                    operation_desc: "config write".to_string(),
                    risk_level: risk,
                    session_resume: "resume".to_string(),
                    callback: Box::new(|_| {
                        panic!("callback should not be invoked on WithWhitelist reject");
                    }),
                },
                &default_rules(),
            )
            .unwrap();

        let result = queue.approve(
            &id,
            ApprovalMode::WithWhitelist {
                target: WhitelistTarget::Auto,
            },
        );
        assert!(
            result.is_err(),
            "ConfigWrite must be rejected with WithWhitelist at risk {:?}",
            risk
        );
        assert_eq!(result.unwrap_err(), RejectWhitelistReason::ConfigWrite);
        // Request must remain pending (not resolved)
        assert!(
            queue.get_pending(&id).is_some(),
            "ConfigWrite request must remain pending at risk {:?}",
            risk
        );
    }
}

#[test]
fn test_config_write_once_mode_still_works() {
    let mut queue = ApprovalQueue::new();
    let id = queue
        .enqueue(
            EnqueueRequest {
                request: PermissionRequestBody::ConfigWrite {
                    agent: "test-agent".to_string(),
                    config_file: "/etc/app.toml".to_string(),
                },
                caller: dummy_caller(),
                operation_desc: "config write".to_string(),
                risk_level: RiskLevel::Low,
                session_resume: "resume".to_string(),
                callback: Box::new(|result| {
                    assert_eq!(result, ApproveOrDeny::Approve);
                }),
            },
            &default_rules(),
        )
        .unwrap();

    let result = queue.approve(&id, ApprovalMode::Once);
    assert!(result.is_ok());
    assert!(result.unwrap());
    assert_eq!(queue.pending_count(), 0);
    assert!(queue.get_pending(&id).is_none());
}

#[test]
fn test_non_config_write_with_whitelist_still_works() {
    let mut queue = ApprovalQueue::new();
    let id = queue
        .enqueue(
            EnqueueRequest {
                request: make_file_op("a"),
                caller: dummy_caller(),
                operation_desc: "file read".to_string(),
                risk_level: RiskLevel::Low,
                session_resume: "resume".to_string(),
                callback: Box::new(|result| {
                    assert_eq!(result, ApproveOrDeny::Approve);
                }),
            },
            &default_rules(),
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
}
