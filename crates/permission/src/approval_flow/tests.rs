use super::*;
use crate::engine::engine_risk::RiskLevel;
use crate::engine::engine_types::{Caller, PermissionRequestBody};
use crate::mock_session_lookup::MockSessionLookup;
use closeclaw_common::{PlanPhase, PlanState, PlanStatus, SessionMode};
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
        Arc::new(|_| {}),
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

// ── Whitelist persistence integration tests (Step 1.5) ─────────────────────

/// Helper: create an ApprovalFlow that writes to a temp dir.
/// Returns (flow, temp_dir_path, whitelist_update_count).
fn flow_with_temp_config_dir() -> (ApprovalFlow, tempfile::TempDir, Arc<AtomicUsize>) {
    let rt = test_runtime();
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
        rt.handle().clone(),
        HeartbeatApprovalMode::default(),
        config_dir,
    );
    (flow, dir, whitelist_count)
}

/// Helper: read the permissions.json for a given agent from a temp config dir.
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

#[test]
fn test_whitelist_mode_persists_rule_to_disk() {
    let (mut flow, dir, _) = flow_with_temp_config_dir();

    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();

    let result = flow.approve_request(&request_id, ApprovalMode::WithWhitelist);
    assert!(result.is_ok());
    assert!(result.unwrap());

    // Wait briefly for the spawned async task to complete.
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Verify the rule was written to permissions.json.
    let rs = read_agent_ruleset(dir.path(), "agent_1");
    assert_eq!(rs.rules.len(), 1, "expected exactly 1 whitelist rule");
    assert_eq!(
        rs.rules[0].effect,
        crate::engine::engine_types::Effect::Allow
    );
    assert!(rs.rules[0].name.starts_with("whitelist-"));
}

#[test]
fn test_once_mode_does_not_persist_whitelist() {
    let (mut flow, dir, _) = flow_with_temp_config_dir();

    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();

    let result = flow.approve_request(&request_id, ApprovalMode::Once);
    assert!(result.is_ok());
    assert!(result.unwrap());

    // Wait briefly for the spawned async task.
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Verify NO permissions.json was written.
    let path = dir
        .path()
        .join("agents")
        .join("agent_1")
        .join("permissions.json");
    assert!(!path.exists(), "Once mode should not write whitelist rules");
}

#[test]
fn test_on_whitelist_updated_callback_invoked() {
    let (mut flow, _dir, whitelist_count) = flow_with_temp_config_dir();

    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();

    let result = flow.approve_request(&request_id, ApprovalMode::WithWhitelist);
    assert!(result.is_ok());
    assert!(result.unwrap());

    // Wait briefly for the spawned async task.
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Verify on_whitelist_updated was called exactly once.
    assert_eq!(
        whitelist_count.load(Ordering::SeqCst),
        1,
        "on_whitelist_updated should be invoked once after successful write"
    );
}

#[test]
fn test_on_whitelist_updated_not_called_for_once_mode() {
    let (mut flow, _dir, whitelist_count) = flow_with_temp_config_dir();

    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();

    let result = flow.approve_request(&request_id, ApprovalMode::Once);
    assert!(result.is_ok());
    assert!(result.unwrap());

    std::thread::sleep(std::time::Duration::from_millis(100));

    assert_eq!(
        whitelist_count.load(Ordering::SeqCst),
        0,
        "on_whitelist_updated should not be called for Once mode"
    );
}

#[test]
fn test_whitelist_write_failure_does_not_block_approval() {
    let rt = test_runtime();
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
        rt.handle().clone(),
        HeartbeatApprovalMode::default(),
        fake_config_dir,
    );

    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();

    // Approval should still succeed despite the write failure.
    let result = flow.approve_request(&request_id, ApprovalMode::WithWhitelist);
    assert!(result.is_ok());
    assert!(
        result.unwrap(),
        "approval should succeed even when write fails"
    );

    std::thread::sleep(std::time::Duration::from_millis(100));

    // on_whitelist_updated should NOT be called when write fails.
    assert_eq!(
        whitelist_count.load(Ordering::SeqCst),
        0,
        "on_whitelist_updated should not be called when write fails"
    );
}

#[test]
fn test_whitelist_persists_only_for_whitelist_mode() {
    // Regression: ensure Once mode never triggers write, WithWhitelist always does.
    let (mut flow, dir, _) = flow_with_temp_config_dir();

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
    let r1 = flow.approve_request(&id1, ApprovalMode::Once);
    assert!(r1.is_ok() && r1.unwrap());
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Second: WithWhitelist mode.
    let r2 = flow.approve_request(&id2, ApprovalMode::WithWhitelist);
    assert!(r2.is_ok() && r2.unwrap());
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Only WithWhitelist should have written a rule.
    let rs = read_agent_ruleset(dir.path(), "agent_1");
    assert_eq!(rs.rules.len(), 1, "only WithWhitelist should persist");
    // The rule should be the CommandExec one (second request).
    match &rs.rules[0].actions[0] {
        crate::engine::engine_types::Action::Command { command, .. } => {
            assert_eq!(command, "ls");
        }
        other => panic!("expected Action::Command, got {:?}", other),
    }
}

#[test]
fn test_whitelist_rule_subject_matches_caller() {
    let (mut flow, dir, _) = flow_with_temp_config_dir();

    let caller = Caller {
        user_id: "ou_alice".to_string(),
        agent: "agent_1".to_string(),
        creator_id: "creator_1".to_string(),
    };
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();

    let result = flow.approve_request(&request_id, ApprovalMode::WithWhitelist);
    assert!(result.is_ok() && result.unwrap());
    std::thread::sleep(std::time::Duration::from_millis(100));

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

// ── Gap 1: approval automatically enters Auto Mode ──────────────────────

/// Verify that approve_request transitions the plan to Executing and
/// switches session mode to Auto, so /execute is no longer required.
#[tokio::test]
async fn test_approve_request_enters_auto_mode() {
    // Keep a concrete Arc so we can call MockSessionLookup-specific methods
    // for setup and assertions, while passing a trait-object Arc to the flow.
    let mock_arc: Arc<MockSessionLookup> = Arc::new(MockSessionLookup::new());
    let sm: Arc<dyn SessionLookup> = mock_arc.clone() as Arc<dyn SessionLookup>;

    // Pre-register a plan state in Confirmed status for session_1.
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
    );

    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();

    let result = flow.approve_request(&request_id, ApprovalMode::Once);
    assert!(result.is_ok());
    assert!(result.unwrap());

    // Wait for the spawned async task to complete.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Verify plan_state was set to Executing.
    let plan = mock_arc.get_tracked_plan_state("session_1");
    assert!(plan.is_some(), "plan_state should be set after approval");
    let plan = plan.unwrap();
    assert_eq!(
        plan.status,
        PlanStatus::Executing,
        "plan status should be Executing after approval"
    );

    // Verify session_mode was switched to Auto.
    let mode = mock_arc.get_tracked_session_mode("session_1");
    assert!(mode.is_some(), "session_mode should be set after approval");
    assert_eq!(
        mode.unwrap(),
        SessionMode::Auto,
        "session mode should be Auto after approval"
    );

    // Verify approval result and mode switch notifications were pushed.
    let msgs = mock_arc.pending_messages();
    assert!(
        msgs.iter()
            .any(|(sid, m)| sid == "session_1" && m.content.contains("已批准")),
        "should have pushed approval result message"
    );
    assert!(
        msgs.iter()
            .any(|(sid, m)| sid == "session_1" && m.content.contains("Auto Mode")),
        "should have pushed mode switch notification"
    );
}
