use super::*;
use crate::approval::WhitelistTarget;
use crate::engine::engine_risk::RiskLevel;
use crate::engine::engine_types::{Caller, PermissionRequestBody, RuleSet};
use crate::mock_session_lookup::MockSessionLookup;
use closeclaw_common::{PlanPhase, PlanState, PlanStatus, SessionMode};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Default whitelist mode for tests.
fn whitelist_auto() -> ApprovalMode {
    ApprovalMode::WithWhitelist {
        target: WhitelistTarget::Auto,
    }
}

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

fn test_approval_flow_with(
    sm: Arc<dyn SessionLookup>,
    notify_count: Arc<AtomicUsize>,
    handle: tokio::runtime::Handle,
) -> ApprovalFlow {
    let nc = Arc::clone(&notify_count);
    ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc.fetch_add(1, Ordering::SeqCst);
        }),
        Arc::new(|_| {}),
        handle,
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
        RuleSet::default(),
    )
}

fn test_approval_flow(
    sm: Arc<dyn SessionLookup>,
    notify_count: Arc<AtomicUsize>,
    rt: &tokio::runtime::Runtime,
) -> ApprovalFlow {
    test_approval_flow_with(sm, notify_count, rt.handle().clone())
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

#[tokio::test]
async fn test_approve_request_once() {
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let mut flow = test_approval_flow_with(
        sm,
        Arc::clone(&notify_count),
        tokio::runtime::Handle::current(),
    );
    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();
    let result = flow.approve_request(&request_id, ApprovalMode::Once).await;
    assert!(result.is_ok());
    assert!(result.unwrap());
}

#[tokio::test]
async fn test_approve_request_whitelist_low_risk() {
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let mut flow = test_approval_flow_with(
        sm,
        Arc::clone(&notify_count),
        tokio::runtime::Handle::current(),
    );
    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();
    let result = flow.approve_request(&request_id, whitelist_auto()).await;
    assert!(result.is_ok());
    assert!(result.unwrap());
}

#[tokio::test]
async fn test_approve_request_whitelist_high_risk() {
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let mut flow = test_approval_flow_with(
        sm,
        Arc::clone(&notify_count),
        tokio::runtime::Handle::current(),
    );
    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::High, "session_1", false)
        .unwrap();
    let result = flow.approve_request(&request_id, whitelist_auto()).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), RejectWhitelistReason::HighRisk);
}

// ── Whitelist persistence integration tests (Step 1.5) ─────────────────────

fn flow_with_temp_config_dir_with(
    handle: tokio::runtime::Handle,
) -> (ApprovalFlow, tempfile::TempDir, Arc<AtomicUsize>) {
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let whitelist_count = Arc::new(AtomicUsize::new(0));
    let wc = Arc::clone(&whitelist_count);
    let dir = tempfile::tempdir().unwrap();
    let config_dir = dir.path().to_path_buf();
    let nc = Arc::clone(&notify_count);
    let flow = ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc.fetch_add(1, Ordering::SeqCst);
        }),
        Arc::new(move |_agent_id: &str| {
            wc.fetch_add(1, Ordering::SeqCst);
        }),
        handle,
        HeartbeatApprovalMode::default(),
        config_dir,
        RuleSet::default(),
    );
    (flow, dir, whitelist_count)
}

fn read_agent_ruleset(
    config_dir: &std::path::Path,
    agent_id: &str,
) -> crate::engine::engine_types::RuleSet {
    let path = config_dir
        .join("agents")
        .join(agent_id)
        .join("permissions.json");
    let data = std::fs::read_to_string(&path).expect("permissions.json should exist");
    serde_json::from_str(&data).expect("valid RuleSet JSON")
}

#[tokio::test]
async fn test_whitelist_mode_persists_rule_to_disk() {
    let (mut flow, dir, _) = flow_with_temp_config_dir_with(tokio::runtime::Handle::current());
    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();
    let result = flow.approve_request(&request_id, whitelist_auto()).await;
    assert!(result.is_ok());
    assert!(result.unwrap());
    tokio::time::sleep(Duration::from_millis(200)).await;
    let rs = read_agent_ruleset(dir.path(), "agent_1");
    assert_eq!(rs.rules.len(), 1, "1 whitelist rule");
    assert_eq!(
        rs.rules[0].effect,
        crate::engine::engine_types::Effect::Allow
    );
    assert!(rs.rules[0].name.starts_with("whitelist-"));
}

#[tokio::test]
async fn test_once_mode_does_not_persist_whitelist() {
    let (mut flow, dir, _) = flow_with_temp_config_dir_with(tokio::runtime::Handle::current());
    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();
    let result = flow.approve_request(&request_id, ApprovalMode::Once).await;
    assert!(result.is_ok());
    assert!(result.unwrap());
    tokio::time::sleep(Duration::from_millis(200)).await;
    let path = dir
        .path()
        .join("agents")
        .join("agent_1")
        .join("permissions.json");
    assert!(!path.exists(), "Once mode should not write whitelist rules");
}

#[tokio::test]
async fn test_on_whitelist_updated_callback_invoked() {
    let (mut flow, _dir, whitelist_count) =
        flow_with_temp_config_dir_with(tokio::runtime::Handle::current());
    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();
    let result = flow.approve_request(&request_id, whitelist_auto()).await;
    assert!(result.is_ok());
    assert!(result.unwrap());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        whitelist_count.load(Ordering::SeqCst),
        1,
        "whitelist callback invoked once"
    );
}

#[tokio::test]
async fn test_whitelist_write_failure_does_not_block_approval() {
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let whitelist_count = Arc::new(AtomicUsize::new(0));
    let wc = Arc::clone(&whitelist_count);
    let nc = Arc::clone(&notify_count);
    // Use a path that is definitely NOT a writable directory
    // (e.g., a non-existent deep path inside a read-only location).
    let fake_config_dir = std::path::PathBuf::from("/nonexistent/path/to/config");
    let mut flow = ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc.fetch_add(1, Ordering::SeqCst);
        }),
        Arc::new(move |_agent_id: &str| {
            wc.fetch_add(1, Ordering::SeqCst);
        }),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        fake_config_dir,
        RuleSet::default(),
    );
    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();
    // Approval should still succeed despite the write failure.
    let result = flow.approve_request(&request_id, whitelist_auto()).await;
    assert!(result.is_ok());
    assert!(result.unwrap(), "approval succeeds despite write failure");
    tokio::time::sleep(Duration::from_millis(200)).await;
    // on_whitelist_updated should NOT be called when write fails.
    assert_eq!(
        whitelist_count.load(Ordering::SeqCst),
        0,
        "no whitelist callback on write failure"
    );
}

#[tokio::test]
async fn test_whitelist_persists_only_for_whitelist_mode() {
    // Regression: ensure Once mode never triggers write, WithWhitelist always does.
    let (mut flow, dir, _) = flow_with_temp_config_dir_with(tokio::runtime::Handle::current());
    let caller = test_caller();
    // Two distinct requests to avoid duplicate rejection.
    let request1 = PermissionRequestBody::FileOp {
        agent: "agent_1".to_string(),
        path: "/tmp/a.txt".to_string(),
        op: "read".to_string(),
    };
    let request2 = PermissionRequestBody::CommandExec {
        agent: "agent_1".to_string(),
        cmd: "ls".to_string(),
        args: vec![],
    };
    let id1 = flow
        .submit_denial(&caller, &request1, RiskLevel::Low, "session_1", false)
        .unwrap();
    let id2 = flow
        .submit_denial(&caller, &request2, RiskLevel::Low, "session_1", false)
        .unwrap();
    // First: Once mode.
    let r1 = flow.approve_request(&id1, ApprovalMode::Once).await;
    assert!(r1.is_ok() && r1.unwrap());
    tokio::time::sleep(Duration::from_millis(200)).await;
    // Second: WithWhitelist mode.
    let r2 = flow.approve_request(&id2, whitelist_auto()).await;
    assert!(r2.is_ok() && r2.unwrap());
    tokio::time::sleep(Duration::from_millis(200)).await;
    // Only WithWhitelist should have written a rule.
    let rs = read_agent_ruleset(dir.path(), "agent_1");
    assert_eq!(rs.rules.len(), 1, "only WithWhitelist persists");
    // The rule should be the CommandExec one (second request).
    match &rs.rules[0].actions[0] {
        crate::engine::engine_types::Action::Command { command, .. } => {
            assert_eq!(command, "ls");
        }
        other => panic!("expected Action::Command, got {:?}", other),
    }
}

#[tokio::test]
async fn test_whitelist_rule_subject_matches_caller() {
    let (mut flow, dir, _) = flow_with_temp_config_dir_with(tokio::runtime::Handle::current());
    let caller = Caller {
        user_id: "ou_alice".to_string(),
        agent: "agent_1".to_string(),
        creator_id: "creator_1".to_string(),
    };
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();
    let result = flow.approve_request(&request_id, whitelist_auto()).await;
    assert!(result.is_ok() && result.unwrap());
    tokio::time::sleep(Duration::from_millis(200)).await;
    let rs = read_agent_ruleset(dir.path(), "agent_1");
    let rule = &rs.rules[0];
    // Non-owner caller with user_id → UserAndAgent subject
    assert!(rule.subject.is_user_and_agent());
    assert_eq!(rule.subject.user_id(), "ou_alice");
    assert_eq!(rule.subject.agent_id(), "agent_1");
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
        Arc::new(|_| {}),
        rt.handle().clone(),
        HeartbeatApprovalMode::Notify,
        std::env::temp_dir(),
        RuleSet::default(),
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
        Arc::new(|_| {}),
        rt.handle().clone(),
        HeartbeatApprovalMode::Ask,
        std::env::temp_dir(),
        RuleSet::default(),
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
        Arc::new(|_| {}),
        rt.handle().clone(),
        HeartbeatApprovalMode::Ask,
        std::env::temp_dir(),
        RuleSet::default(),
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
            Arc::new(|_| {}),
            rt.handle().clone(),
            mode,
            std::env::temp_dir(),
            RuleSet::default(),
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
        Arc::new(|_| {}),
        rt.handle().clone(),
        HeartbeatApprovalMode::Skip,
        std::env::temp_dir(),
        RuleSet::default(),
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

/// Verify that approve_request transitions the plan to Executing and
/// switches session mode to Auto, so /execute is no longer required.
#[tokio::test]
async fn test_approve_request_enters_auto_mode() {
    let mock_arc: Arc<MockSessionLookup> = Arc::new(MockSessionLookup::new());
    let sm: Arc<dyn SessionLookup> = mock_arc.clone() as Arc<dyn SessionLookup>;
    let initial_plan = PlanState {
        phase: PlanPhase::FinalPlan,
        status: PlanStatus::Confirmed,
        plan_file_path: "/tmp/test-plan.md".to_string(),
        ..PlanState::new()
    };
    mock_arc.set_plan_state("session_1", initial_plan).await;
    let nc = Arc::new(AtomicUsize::new(0));
    let nc_clone = Arc::clone(&nc);
    let mut flow = ApprovalFlow::new(
        sm,
        Arc::new(move |_n: ApprovalNotification| {
            nc_clone.fetch_add(1, Ordering::SeqCst);
        }),
        Arc::new(|_| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
        RuleSet::default(),
    );
    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();
    let result = flow.approve_request(&request_id, ApprovalMode::Once).await;
    assert!(result.is_ok());
    assert!(result.unwrap());
    tokio::time::sleep(Duration::from_millis(200)).await;
    let plan = mock_arc.get_tracked_plan_state("session_1");
    assert!(plan.is_some(), "plan_state set");
    let plan = plan.unwrap();
    assert_eq!(plan.status, PlanStatus::Executing, "plan Executing");
    let mode = mock_arc.get_tracked_session_mode("session_1");
    assert!(mode.is_some(), "session_mode set");
    assert_eq!(mode.unwrap(), SessionMode::Auto, "session Auto");
    let msgs = mock_arc.pending_messages();
    assert!(
        msgs.iter()
            .any(|(sid, m)| sid == "session_1" && m.content.contains("已批准")),
        "approval msg pushed"
    );
    assert!(
        msgs.iter()
            .any(|(sid, m)| sid == "session_1" && m.content.contains("Auto Mode")),
        "mode switch pushed"
    );
}

// ── User creation approval flow tests (Step 1.6) ───────────────────────

#[test]
fn test_submit_user_creation_dedup() {
    let rt = test_runtime();
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let mut flow = test_approval_flow(sm, Arc::clone(&notify_count), &rt);
    let id1 = flow.submit_user_creation("ou_dup", "feishu", vec![]);
    assert!(id1.is_some());
    // Same user+channel is a duplicate.
    let id2 = flow.submit_user_creation("ou_dup", "feishu", vec![]);
    assert!(id2.is_none());
    assert_eq!(notify_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_approve_user_creation_persists_rules() {
    let (mut flow, dir, _) = flow_with_temp_config_dir_with(tokio::runtime::Handle::current());
    let request_id = flow
        .submit_user_creation(
            "ou_approved",
            "feishu",
            vec![closeclaw_common::permission_op::InitialPermissionSet::BasicMessaging],
        )
        .unwrap();
    let result = flow.approve_request(&request_id, ApprovalMode::Once).await;
    assert!(result.is_ok());
    assert!(result.unwrap());
    tokio::time::sleep(Duration::from_millis(200)).await;
    let registry_path = dir.path().join("users.json");
    assert!(registry_path.exists(), "users.json should be created");
    let data = std::fs::read_to_string(&registry_path).unwrap();
    let registry: crate::UserRegistry = serde_json::from_str(&data).unwrap();
    assert_eq!(registry.list_users().len(), 1);
    assert_eq!(registry.list_users()[0].user_id, "ou_approved");
    assert_eq!(registry.list_users()[0].im_channel, "feishu");
    let ruleset = read_agent_ruleset(dir.path(), "ou_approved");
    assert_eq!(
        ruleset.rules.len(),
        2,
        "should have 2 initial permission rules"
    );
}

#[tokio::test]
async fn test_approve_user_creation_duplicate_user_fails() {
    let (mut flow, _dir, _) = flow_with_temp_config_dir_with(tokio::runtime::Handle::current());
    let id1 = flow
        .submit_user_creation("ou_dup_user", "feishu", vec![])
        .unwrap();
    let result1 = flow.approve_request(&id1, ApprovalMode::Once).await;
    assert!(result1.is_ok() && result1.unwrap());
    tokio::time::sleep(Duration::from_millis(200)).await;
    // Second request for same user should fail registration.
    let id2 = flow
        .submit_user_creation("ou_dup_user", "feishu", vec![])
        .unwrap();
    let result2 = flow.approve_request(&id2, ApprovalMode::Once).await;
    assert!(result2.is_ok());
    assert!(
        !result2.unwrap(),
        "duplicate registration should return false"
    );
}

#[tokio::test]
async fn test_user_creation_initial_permissions_set() {
    let (mut flow, dir, _) = flow_with_temp_config_dir_with(tokio::runtime::Handle::current());
    let request_id = flow
        .submit_user_creation(
            "ou_perms",
            "feishu",
            vec![closeclaw_common::permission_op::InitialPermissionSet::BasicMessaging],
        )
        .unwrap();
    flow.approve_request(&request_id, ApprovalMode::Once)
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    let registry_path = dir.path().join("users.json");
    let data = std::fs::read_to_string(&registry_path).unwrap();
    let registry: crate::UserRegistry = serde_json::from_str(&data).unwrap();
    let user = &registry.list_users()[0];
    assert_eq!(
        user.initial_permissions,
        vec![closeclaw_common::permission_op::InitialPermissionSet::BasicMessaging]
    );
}

#[tokio::test]
async fn test_user_creation_request_removed_after_approve() {
    let (mut flow, _dir, _) = flow_with_temp_config_dir_with(tokio::runtime::Handle::current());

    let request_id = flow
        .submit_user_creation("ou_removed", "feishu", vec![])
        .unwrap();

    let result = flow.approve_request(&request_id, ApprovalMode::Once).await;
    assert!(result.is_ok() && result.unwrap());

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Second approve on same request_id should fail (no longer pending).
    let result2 = flow.approve_request(&request_id, ApprovalMode::Once).await;
    assert!(result2.is_ok());
    assert!(
        !result2.unwrap(),
        "already resolved request should return false"
    );
}

// ── Step 1.7: Rule version snapshot integration tests ──────────────────────

use crate::engine::engine_types::{Effect, Rule, Subject};

fn make_rules(effect: Effect, agent: &str) -> RuleSet {
    let mut rules = RuleSet::default();
    rules.rules.push(Rule {
        name: format!(
            "{}-test-tool",
            if effect == Effect::Deny {
                "deny"
            } else {
                "allow"
            }
        ),
        subject: Subject::AgentOnly {
            agent: agent.to_string(),
            match_type: Default::default(),
        },
        effect,
        actions: vec![crate::engine::engine_types::Action::ToolCall {
            skill: "test_skill".to_string(),
            methods: vec!["test_method".to_string()],
        }],
        template: None,
        priority: 0,
    });
    if effect == Effect::Allow {
        rules.user_defaults.tool_call = Effect::Allow;
    }
    rules.compute_version();
    rules
}

fn flow_with(initial_rules: RuleSet) -> ApprovalFlow {
    ApprovalFlow::new(
        Arc::new(MockSessionLookup::new()),
        Arc::new(|_: ApprovalNotification| {}),
        Arc::new(|_| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
        initial_rules,
    )
}

#[tokio::test]
async fn test_rule_version_snapshot_uses_snapshotted_rules() {
    let deny = make_rules(Effect::Deny, "agent_1");
    let allow = make_rules(Effect::Allow, "agent_1");
    let caller = test_caller();
    let request = test_request();
    let mut flow = flow_with(deny.clone());
    let rid = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "s1", false)
        .unwrap();
    let p = flow.queue.get_pending(&rid).unwrap();
    assert_eq!(p.rule_version, deny.rule_version);
    assert_eq!(p.snapshotted_rules.rule_version, deny.rule_version);
    flow.update_rules(allow);
    let r = flow.approve_request(&rid, ApprovalMode::Once).await;
    assert!(r.is_ok() && r.unwrap());
    assert!(flow.queue.get_pending(&rid).is_none());
}

#[tokio::test]
async fn test_window_period_protection_old_rules_apply() {
    let deny = make_rules(Effect::Deny, "agent_1");
    let allow = make_rules(Effect::Allow, "agent_1");
    let caller = test_caller();
    let request = test_request();
    let mut flow = flow_with(deny.clone());
    let rid = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "s1", false)
        .unwrap();
    let old_ver = deny.rule_version.clone();
    assert_eq!(flow.queue.get_pending(&rid).unwrap().rule_version, old_ver);
    flow.update_rules(allow);
    assert_ne!(flow.current_rules.rule_version, old_ver);
    let r = flow.approve_request(&rid, whitelist_auto()).await;
    assert!(r.is_ok() && r.unwrap());
}

#[tokio::test]
async fn test_reevaluation_auto_approve_when_snapshot_allows() {
    let allow = make_rules(Effect::Allow, "agent_1");
    let deny = make_rules(Effect::Deny, "agent_1");
    let caller = test_caller();
    let request = test_request();
    let mut flow = ApprovalFlow::new_deny_all(
        Arc::new(MockSessionLookup::new()),
        Arc::new(|_: ApprovalNotification| {}),
        Arc::new(|_| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
        allow.clone(),
    );
    let rid = flow
        .queue
        .enqueue(
            crate::approval::EnqueueRequest {
                request: request.clone(),
                caller: caller.clone(),
                operation_desc: "test op".to_string(),
                risk_level: RiskLevel::Low,
                session_resume: "s1".to_string(),
                callback: Box::new(|_| {}),
            },
            &flow.current_rules,
        )
        .unwrap();
    let p = flow.queue.get_pending(&rid).unwrap();
    assert_eq!(p.rule_version, allow.rule_version);
    let engine = crate::engine::engine_eval::PermissionEngine::new_with_default_data_root(
        p.snapshotted_rules.clone(),
    );
    let pr = crate::engine::engine_types::PermissionRequest::WithCaller {
        caller: caller.clone(),
        request: request.clone(),
    };
    assert!(matches!(
        engine.evaluate(pr, None),
        crate::engine::engine_types::PermissionResponse::Allowed { .. }
    ));
    flow.update_rules(deny);
    let r = flow.approve_request(&rid, ApprovalMode::Once).await;
    assert!(r.is_ok() && r.unwrap());
    assert!(flow.queue.get_pending(&rid).is_none());
}

#[tokio::test]
async fn test_pending_approval_stores_rule_version_and_snapshot() {
    let deny = make_rules(Effect::Deny, "agent_1");
    let caller = test_caller();
    let request = test_request();
    let mut flow = flow_with(deny.clone());
    let rid = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "s1", false)
        .unwrap();
    let p = flow.queue.get_pending(&rid).unwrap();
    assert_eq!(p.rule_version, deny.rule_version);
    assert_eq!(p.snapshotted_rules.rules.len(), 1);
    assert_eq!(p.snapshotted_rules.rules[0].effect, Effect::Deny);
    assert_eq!(p.snapshotted_rules.rules[0].name, "deny-test-tool");
    assert_eq!(p.rule_version.len(), 64);
}

// ── New session execution path tests (Step 1.8) ─────────────────────────

/// Helper: create a tempfile-backed approval flow with plan state.
async fn ns_flow() -> (
    tempfile::TempDir,
    Arc<MockSessionLookup>,
    ApprovalFlow,
    String,
) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("plan.md");
    std::fs::write(&p, "# Plan\n").unwrap();
    let ps = p.to_str().unwrap().to_string();
    let m: Arc<MockSessionLookup> = Arc::new(MockSessionLookup::new());
    m.set_plan_state(
        "s1",
        PlanState {
            phase: PlanPhase::FinalPlan,
            status: PlanStatus::Confirmed,
            plan_file_path: ps.clone(),
            ..PlanState::new()
        },
    )
    .await;
    let nc = Arc::new(AtomicUsize::new(0));
    (
        d,
        m.clone(),
        test_approval_flow_with(m, nc, tokio::runtime::Handle::current()),
        ps,
    )
}

#[tokio::test]
async fn test_new_session_creates_child() {
    let (d, m, mut f, ps) = ns_flow().await;
    let c = Arc::new(AtomicUsize::new(0));
    let cc = Arc::clone(&c);
    f.set_create_child_session_fn(Arc::new(
        move |_: String, _: String, _: Option<Vec<usize>>| {
            let cc = Arc::clone(&cc);
            Box::pin(async move {
                cc.fetch_add(1, Ordering::SeqCst);
                Ok("c1".to_string())
            })
        },
    ));
    let rid = f
        .submit_denial(&test_caller(), &test_request(), RiskLevel::Low, "s1", false)
        .unwrap();
    f.set_plan_exec_metadata(&rid, ps, Some(vec![0, 2]), true);
    assert!(f.approve_request(&rid, ApprovalMode::Once).await.unwrap());
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(c.load(Ordering::SeqCst), 1);
    assert_eq!(m.get_tracked_session_mode("c1").unwrap(), SessionMode::Auto);
    let p = m.get_tracked_plan_state("c1").unwrap();
    assert_eq!(p.status, PlanStatus::Executing);
    assert_eq!(p.step_selection, Some(vec![0, 2]));
    drop(d);
}

#[tokio::test]
async fn test_new_session_fallback_without_callback() {
    let (d, m, mut f, ps) = ns_flow().await;
    let rid = f
        .submit_denial(&test_caller(), &test_request(), RiskLevel::Low, "s1", false)
        .unwrap();
    f.set_plan_exec_metadata(&rid, ps, None, true);
    assert!(f.approve_request(&rid, ApprovalMode::Once).await.unwrap());
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        m.get_tracked_plan_state("s1").unwrap().phase,
        PlanPhase::FinalPlan
    );
    assert!(m.get_tracked_plan_state("c1").is_none());
    drop(d);
}

#[tokio::test]
async fn test_new_session_step_selection_metadata() {
    let (d, m, mut f, ps) = ns_flow().await;
    let rid = f
        .submit_denial(&test_caller(), &test_request(), RiskLevel::Low, "s1", false)
        .unwrap();
    f.set_plan_exec_metadata(&rid, ps, Some(vec![1, 3]), false);
    assert!(f.approve_request(&rid, ApprovalMode::Once).await.unwrap());
    tokio::time::sleep(Duration::from_millis(500)).await;
    let p = m.get_tracked_plan_state("s1").unwrap();
    assert_eq!(p.step_selection, Some(vec![1, 3]));
    assert_eq!(p.status, PlanStatus::Executing);
    drop(d);
}
