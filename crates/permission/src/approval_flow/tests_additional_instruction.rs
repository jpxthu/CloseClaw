//! Additional instruction injection tests for plan execution approval flow.

use super::super::*;
use crate::engine::engine_risk::RiskLevel;
use crate::mock_session_lookup::MockSessionLookup;
use closeclaw_common::{PlanPhase, PlanState};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn test_caller() -> Caller {
    Caller {
        user_id: "user_1".to_string(),
        agent: "agent_1".to_string(),
    }
}

fn test_request() -> PermissionRequestBody {
    PermissionRequestBody::ToolCall {
        agent: "agent_1".to_string(),
        skill: "test_skill".to_string(),
        method: "test_method".to_string(),
    }
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

/// Helper for same-session plan execution tests.
async fn ss_flow() -> (
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

/// Helper for new-session plan execution tests.
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
async fn test_same_session_additional_instruction_injected() {
    let (d, m, mut f, ps) = ss_flow().await;
    let rid = f
        .submit_denial(&test_caller(), &test_request(), RiskLevel::Low, "s1", false)
        .unwrap();
    f.set_plan_exec_metadata(
        &rid,
        ps,
        None,
        false,
        Some("请优先处理测试用例".to_string()),
    );
    assert!(f.approve_request(&rid, ApprovalMode::Once).await.unwrap());
    tokio::time::sleep(Duration::from_millis(500)).await;
    let msgs = m.pending_messages();
    let instruction_msgs: Vec<_> = msgs
        .iter()
        .filter(|(sid, msg)| sid == "s1" && msg.role.as_deref() == Some("user"))
        .collect();
    assert_eq!(
        instruction_msgs.len(),
        1,
        "should have exactly one user-role pending message for the instruction"
    );
    assert_eq!(instruction_msgs[0].1.content, "请优先处理测试用例");
    drop(d);
}

#[tokio::test]
async fn test_same_session_no_additional_instruction() {
    let (d, m, mut f, ps) = ss_flow().await;
    let rid = f
        .submit_denial(&test_caller(), &test_request(), RiskLevel::Low, "s1", false)
        .unwrap();
    f.set_plan_exec_metadata(&rid, ps, None, false, None);
    assert!(f.approve_request(&rid, ApprovalMode::Once).await.unwrap());
    tokio::time::sleep(Duration::from_millis(500)).await;
    let msgs = m.pending_messages();
    let user_msgs: Vec<_> = msgs
        .iter()
        .filter(|(sid, msg)| sid == "s1" && msg.role.as_deref() == Some("user"))
        .collect();
    assert!(
        user_msgs.is_empty(),
        "should have no user-role messages when no additional instruction"
    );
    drop(d);
}

#[tokio::test]
async fn test_new_session_additional_instruction_injected() {
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
    f.set_plan_exec_metadata(
        &rid,
        ps,
        None,
        true,
        Some("在新 session 中执行此 plan".to_string()),
    );
    assert!(f.approve_request(&rid, ApprovalMode::Once).await.unwrap());
    tokio::time::sleep(Duration::from_millis(500)).await;
    let msgs = m.pending_messages();
    let child_instruction_msgs: Vec<_> = msgs
        .iter()
        .filter(|(sid, msg)| sid == "c1" && msg.role.as_deref() == Some("user"))
        .collect();
    assert_eq!(
        child_instruction_msgs.len(),
        1,
        "child session should have one user-role message for the instruction"
    );
    assert_eq!(
        child_instruction_msgs[0].1.content,
        "在新 session 中执行此 plan"
    );
    drop(d);
}

#[tokio::test]
async fn test_additional_instruction_whitespace_only_not_injected() {
    let (d, m, mut f, ps) = ss_flow().await;
    let rid = f
        .submit_denial(&test_caller(), &test_request(), RiskLevel::Low, "s1", false)
        .unwrap();
    f.set_plan_exec_metadata(&rid, ps, None, false, Some("   ".to_string()));
    assert!(f.approve_request(&rid, ApprovalMode::Once).await.unwrap());
    tokio::time::sleep(Duration::from_millis(500)).await;
    let msgs = m.pending_messages();
    let user_msgs: Vec<_> = msgs
        .iter()
        .filter(|(sid, msg)| sid == "s1" && msg.role.as_deref() == Some("user"))
        .collect();
    assert!(
        user_msgs.is_empty(),
        "whitespace-only instruction should not be injected"
    );
    drop(d);
}
