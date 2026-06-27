use super::*;
use crate::engine::engine_risk::RiskLevel;
use crate::engine::engine_types::{Caller, PermissionRequestBody};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Minimal test helper: creates a SessionManager with no storage.
fn test_session_manager() -> Arc<SessionManager> {
    use crate::gateway::{DmScope, GatewayConfig};
    use crate::session::bootstrap::loader::BootstrapMode;
    use crate::session::persistence::ReasoningLevel;

    let config = GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        dm_scope: DmScope::PerChannelPeer,
        ..Default::default()
    };
    Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    ))
}

fn test_caller() -> Caller {
    Caller {
        user_id: "user_1".to_string(),
        agent: "agent_1".to_string(),
        creator_id: "creator_1".to_string(),
    }
}

fn test_request() -> PermissionRequestBody {
    PermissionRequestBody::ToolCall {
        agent: "agent_1".to_string(),
        skill: "test_skill".to_string(),
        method: "test_method".to_string(),
    }
}

fn test_heartbeat_request() -> PermissionRequestBody {
    PermissionRequestBody::ToolCall {
        agent: "agent_1".to_string(),
        skill: "heartbeat".to_string(),
        method: "ping".to_string(),
    }
}

fn test_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}

#[test]
fn test_submit_denial_enqueues_and_notifies() {
    let rt = test_runtime();
    let sm = test_session_manager();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let nc = Arc::clone(&notify_count);

    let mut flow = ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc.fetch_add(1, Ordering::SeqCst);
        }),
        rt.handle().clone(),
    );

    let caller = test_caller();
    let request = test_request();

    let result = flow.submit_denial(&caller, &request, RiskLevel::Low, "session_1", false);
    assert!(result.is_some());
    assert_eq!(notify_count.load(Ordering::SeqCst), 1);
}

#[test]
fn test_submit_denial_sub_agent_returns_none() {
    let rt = test_runtime();
    let sm = test_session_manager();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let nc = Arc::clone(&notify_count);

    let mut flow = ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc.fetch_add(1, Ordering::SeqCst);
        }),
        rt.handle().clone(),
    );

    let caller = test_caller();
    let request = test_request();

    let result = flow.submit_denial(&caller, &request, RiskLevel::Low, "session_1", true);
    assert!(result.is_none());
    assert_eq!(notify_count.load(Ordering::SeqCst), 0);
}

#[test]
fn test_submit_denial_heartbeat_skip_returns_none() {
    let rt = test_runtime();
    let sm = test_session_manager();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let nc = Arc::clone(&notify_count);

    let mut flow = ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc.fetch_add(1, Ordering::SeqCst);
        }),
        rt.handle().clone(),
    );

    let caller = test_caller();
    let request = test_heartbeat_request();

    let result = flow.submit_denial(&caller, &request, RiskLevel::Low, "session_1", false);
    assert!(result.is_none());
    assert_eq!(notify_count.load(Ordering::SeqCst), 0);
}

#[test]
fn test_duplicate_denial_returns_none() {
    let rt = test_runtime();
    let sm = test_session_manager();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let nc = Arc::clone(&notify_count);

    let mut flow = ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc.fetch_add(1, Ordering::SeqCst);
        }),
        rt.handle().clone(),
    );

    let caller = test_caller();
    let request = test_request();

    // First enqueue succeeds.
    let result1 = flow.submit_denial(&caller, &request, RiskLevel::Low, "session_1", false);
    assert!(result1.is_some());

    // Duplicate is silently rejected.
    let result2 = flow.submit_denial(&caller, &request, RiskLevel::Low, "session_1", false);
    assert!(result2.is_none());

    // Only one notification was sent.
    assert_eq!(notify_count.load(Ordering::SeqCst), 1);
}

#[test]
fn test_approve_request_once() {
    let rt = test_runtime();
    let sm = test_session_manager();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let nc = Arc::clone(&notify_count);

    let mut flow = ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc.fetch_add(1, Ordering::SeqCst);
        }),
        rt.handle().clone(),
    );

    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();

    let result = flow.approve_request(&request_id, ApprovalMode::Once);
    assert!(result.is_ok());
    assert!(result.unwrap());
}

#[test]
fn test_approve_request_whitelist_low_risk() {
    let rt = test_runtime();
    let sm = test_session_manager();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let nc = Arc::clone(&notify_count);

    let mut flow = ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc.fetch_add(1, Ordering::SeqCst);
        }),
        rt.handle().clone(),
    );

    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();

    let result = flow.approve_request(&request_id, ApprovalMode::WithWhitelist);
    assert!(result.is_ok());
    assert!(result.unwrap());
}

#[test]
fn test_approve_request_whitelist_high_risk() {
    let rt = test_runtime();
    let sm = test_session_manager();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let nc = Arc::clone(&notify_count);

    let mut flow = ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc.fetch_add(1, Ordering::SeqCst);
        }),
        rt.handle().clone(),
    );

    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::High, "session_1", false)
        .unwrap();

    let result = flow.approve_request(&request_id, ApprovalMode::WithWhitelist);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), RejectWhitelistReason::HighRisk);
}

#[test]
fn test_deny_request() {
    let rt = test_runtime();
    let sm = test_session_manager();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let nc = Arc::clone(&notify_count);

    let mut flow = ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc.fetch_add(1, Ordering::SeqCst);
        }),
        rt.handle().clone(),
    );

    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();

    let result = flow.deny_request(&request_id);
    assert!(result);
}

#[test]
fn test_clear() {
    let rt = test_runtime();
    let sm = test_session_manager();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let nc = Arc::clone(&notify_count);

    let mut flow = ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc.fetch_add(1, Ordering::SeqCst);
        }),
        rt.handle().clone(),
    );

    let caller = test_caller();
    let request = test_request();

    // Enqueue two different requests.
    let id1 = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();

    let mut request2 = test_request();
    if let PermissionRequestBody::ToolCall { method, .. } = &mut request2 {
        *method = "other_method".to_string();
    }
    let id2 = flow
        .submit_denial(&caller, &request2, RiskLevel::Low, "session_1", false)
        .unwrap();

    // Clear all — should trigger deny callbacks.
    flow.clear();

    // The queue should be empty; further approvals/denials should fail.
    assert!(!flow.deny_request(&id1));
    assert!(!flow.deny_request(&id2));
}
