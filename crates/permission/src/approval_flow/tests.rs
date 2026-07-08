use super::*;
use crate::engine::engine_risk::RiskLevel;
use crate::engine::engine_types::{Caller, PermissionRequestBody};
use crate::mock_session_lookup::MockSessionLookup;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Minimal test helper: creates a MockSessionLookup.
fn test_session_lookup() -> Arc<dyn SessionLookup> {
    Arc::new(MockSessionLookup::new())
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

fn test_approval_flow(
    sm: Arc<dyn SessionLookup>,
    notify_count: Arc<AtomicUsize>,
    rt: &tokio::runtime::Runtime,
) -> ApprovalFlow {
    let nc = Arc::clone(&notify_count);
    ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc.fetch_add(1, Ordering::SeqCst);
        }),
        rt.handle().clone(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
    )
}

#[test]
fn test_submit_denial_enqueues_and_notifies() {
    let rt = test_runtime();
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let mut flow = test_approval_flow(sm, Arc::clone(&notify_count), &rt);

    let caller = test_caller();
    let request = test_request();

    let result = flow.submit_denial(&caller, &request, RiskLevel::Low, "session_1", false);
    assert!(result.is_some());
    assert_eq!(notify_count.load(Ordering::SeqCst), 1);
}

#[test]
fn test_submit_denial_sub_agent_returns_none() {
    let rt = test_runtime();
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let mut flow = test_approval_flow(sm, Arc::clone(&notify_count), &rt);

    let caller = test_caller();
    let request = test_request();

    let result = flow.submit_denial(&caller, &request, RiskLevel::Low, "session_1", true);
    assert!(result.is_none());
    assert_eq!(notify_count.load(Ordering::SeqCst), 0);
}

#[test]
fn test_submit_denial_heartbeat_skip_returns_none() {
    let rt = test_runtime();
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let mut flow = test_approval_flow(sm, Arc::clone(&notify_count), &rt);

    let caller = test_caller();
    let request = test_heartbeat_request();

    let result = flow.submit_denial(&caller, &request, RiskLevel::Low, "session_1", false);
    assert!(result.is_none());
    assert_eq!(notify_count.load(Ordering::SeqCst), 0);
}

#[test]
fn test_duplicate_denial_returns_none() {
    let rt = test_runtime();
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let mut flow = test_approval_flow(sm, Arc::clone(&notify_count), &rt);

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
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let mut flow = test_approval_flow(sm, Arc::clone(&notify_count), &rt);

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
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let mut flow = test_approval_flow(sm, Arc::clone(&notify_count), &rt);

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
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let mut flow = test_approval_flow(sm, Arc::clone(&notify_count), &rt);

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
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let mut flow = test_approval_flow(sm, Arc::clone(&notify_count), &rt);

    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();

    let result = flow.deny_request(&request_id);
    assert!(result);
}

#[test]
fn test_heartbeat_notify_returns_none_and_notifies() {
    let rt = test_runtime();
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let nc = Arc::clone(&notify_count);
    let mut flow = ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc.fetch_add(1, Ordering::SeqCst);
        }),
        rt.handle().clone(),
        HeartbeatApprovalMode::Notify,
        std::env::temp_dir(),
    );

    let caller = test_caller();
    let request = test_heartbeat_request();

    let result = flow.submit_denial(&caller, &request, RiskLevel::Low, "session_1", false);
    // Notify mode: returns None, one notification sent.
    assert!(result.is_none());
    assert_eq!(notify_count.load(Ordering::SeqCst), 1);
}

#[test]
fn test_heartbeat_ask_enqueues_and_notifies() {
    let rt = test_runtime();
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let nc = Arc::clone(&notify_count);
    let mut flow = ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc.fetch_add(1, Ordering::SeqCst);
        }),
        rt.handle().clone(),
        HeartbeatApprovalMode::Ask,
        std::env::temp_dir(),
    );

    let caller = test_caller();
    let request = test_heartbeat_request();

    let result = flow.submit_denial(&caller, &request, RiskLevel::Low, "session_1", false);
    // Ask mode: returns Some(request_id), one notification sent.
    assert!(result.is_some());
    assert_eq!(notify_count.load(Ordering::SeqCst), 1);
}

#[test]
fn test_heartbeat_ask_dedup() {
    let rt = test_runtime();
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let nc = Arc::clone(&notify_count);
    let mut flow = ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc.fetch_add(1, Ordering::SeqCst);
        }),
        rt.handle().clone(),
        HeartbeatApprovalMode::Ask,
        std::env::temp_dir(),
    );

    let caller = test_caller();
    let request = test_heartbeat_request();

    // First submission succeeds.
    let result1 = flow.submit_denial(&caller, &request, RiskLevel::Low, "session_1", false);
    assert!(result1.is_some());

    // Duplicate is rejected.
    let result2 = flow.submit_denial(&caller, &request, RiskLevel::Low, "session_1", false);
    assert!(result2.is_none());

    // Only one notification was sent.
    assert_eq!(notify_count.load(Ordering::SeqCst), 1);
}

#[test]
fn test_non_heartbeat_unaffected_by_heartbeat_mode() {
    let rt = test_runtime();
    let caller = test_caller();
    let request = test_request();

    for mode in [
        HeartbeatApprovalMode::Skip,
        HeartbeatApprovalMode::Notify,
        HeartbeatApprovalMode::Ask,
    ] {
        let nc = Arc::new(AtomicUsize::new(0));
        let nc_clone = Arc::clone(&nc);
        let sm_clone: Arc<dyn SessionLookup> = Arc::new(MockSessionLookup::new());
        let mut flow = ApprovalFlow::new(
            sm_clone,
            Arc::new(move |_n: ApprovalNotification| {
                nc_clone.fetch_add(1, Ordering::SeqCst);
            }),
            rt.handle().clone(),
            mode,
            std::env::temp_dir(),
        );

        let result = flow.submit_denial(&caller, &request, RiskLevel::Low, "session_1", false);
        // Non-heartbeat always enqueues regardless of heartbeat_mode.
        assert!(
            result.is_some(),
            "non-heartbeat should enqueue with mode {:?}",
            mode
        );
        assert_eq!(nc.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn test_set_heartbeat_mode_runtime_switch() {
    let rt = test_runtime();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let nc = Arc::clone(&notify_count);
    let sm: Arc<dyn SessionLookup> = Arc::new(MockSessionLookup::new());
    let mut flow = ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc.fetch_add(1, Ordering::SeqCst);
        }),
        rt.handle().clone(),
        HeartbeatApprovalMode::Skip,
        std::env::temp_dir(),
    );

    let caller = test_caller();
    let request = test_heartbeat_request();

    // Initially Skip: returns None, no notification.
    let result = flow.submit_denial(&caller, &request, RiskLevel::Low, "session_1", false);
    assert!(result.is_none());
    assert_eq!(notify_count.load(Ordering::SeqCst), 0);

    // Switch to Notify at runtime.
    flow.set_heartbeat_mode(HeartbeatApprovalMode::Notify);
    let result = flow.submit_denial(&caller, &request, RiskLevel::Low, "session_1", false);
    // Notify: returns None, notification sent.
    assert!(result.is_none());
    assert_eq!(notify_count.load(Ordering::SeqCst), 1);

    // Switch to Ask at runtime.
    flow.set_heartbeat_mode(HeartbeatApprovalMode::Ask);
    let result = flow.submit_denial(&caller, &request, RiskLevel::Low, "session_1", false);
    // Ask: returns Some, notification sent.
    assert!(result.is_some());
    assert_eq!(notify_count.load(Ordering::SeqCst), 2);
}

#[test]
fn test_clear() {
    let rt = test_runtime();
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let mut flow = test_approval_flow(sm, Arc::clone(&notify_count), &rt);

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
