//! Unit tests for the daemon
//! [`SessionWorkflowOps`](crate::workflow_session_ops_adapter::SessionWorkflowOps)
//! adapter over the gateway `SessionManager`.

use std::sync::Arc;

use super::*;

use closeclaw_gateway::GatewayConfig;
use closeclaw_workflow::run::{GoalHint, PendingVerify, Phase, WorkflowRun};

fn make_session_manager() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        &GatewayConfig::default(),
        None,
        None,
        closeclaw_common::ReasoningLevel::default(),
    ))
}

async fn insert_session(mgr: &Arc<SessionManager>, session_id: &str) {
    mgr.conversation_sessions.write().await.insert(
        session_id.to_string(),
        Arc::new(tokio::sync::RwLock::new(ConversationSession::new(
            session_id.to_string(),
            "stub-model".to_string(),
            std::env::temp_dir().join("workflow-ops-tests"),
        ))),
    );
}

/// Serialized run snapshot in the given phase with the given pause reason.
fn run_value(phase: Phase, paused_reason: &str) -> serde_json::Value {
    let run = WorkflowRun {
        workflow_id: "run-1".into(),
        definition_name: "test-workflow".into(),
        definition_version: "0.1".into(),
        current_step: 0,
        phase,
        current_step_entered_at: String::new(),
        step_history: Vec::new(),
        step_data: serde_yaml::Value::Null,
        pending_goal_hint: GoalHint::Normal,
        pending_verify: PendingVerify::default(),
        paused_reason: paused_reason.to_string(),
    };
    serde_json::to_value(&run).unwrap()
}

async fn set_run(mgr: &Arc<SessionManager>, session_id: &str, value: serde_json::Value) {
    let cs = mgr
        .get_conversation_session(session_id)
        .await
        .expect("session inserted");
    let mut session = cs.write().await;
    session.set_workflow_port(crate::workflow_port_adapter::engine_workflow_port());
    session.set_workflow_run_value(Some(value));
}

// ── owner_pause_kind（暂停类型判定） ────────────────────────────────

#[tokio::test]
async fn test_missing_session_classifies_as_not_blocked() {
    let mgr = make_session_manager();
    let ops = session_workflow_ops(mgr);
    assert_eq!(
        ops.owner_pause_kind("no-such-session").await,
        OwnerPauseKind::NotBlocked
    );
}

#[tokio::test]
async fn test_no_run_classifies_as_not_blocked() {
    let mgr = make_session_manager();
    insert_session(&mgr, "s1").await;
    let ops = session_workflow_ops(mgr);
    assert_eq!(ops.owner_pause_kind("s1").await, OwnerPauseKind::NotBlocked);
}

#[tokio::test]
async fn test_running_phase_classifies_as_not_blocked() {
    let mgr = make_session_manager();
    insert_session(&mgr, "s1").await;
    set_run(&mgr, "s1", run_value(Phase::Executing, "")).await;
    let ops = session_workflow_ops(mgr);
    assert_eq!(ops.owner_pause_kind("s1").await, OwnerPauseKind::NotBlocked);
}

#[tokio::test]
async fn test_definition_change_pause_is_terminable() {
    let mgr = make_session_manager();
    insert_session(&mgr, "s1").await;
    set_run(
        &mgr,
        "s1",
        run_value(Phase::Blocked, DEFINITION_CHANGED_PAUSE_REASON),
    )
    .await;
    let ops = session_workflow_ops(mgr);
    assert_eq!(ops.owner_pause_kind("s1").await, OwnerPauseKind::Terminable);
}

#[tokio::test]
async fn test_other_pause_reason_is_resolvable() {
    let mgr = make_session_manager();
    insert_session(&mgr, "s1").await;
    set_run(&mgr, "s1", run_value(Phase::Blocked, "验收重试次数耗尽")).await;
    let ops = session_workflow_ops(mgr);
    assert_eq!(ops.owner_pause_kind("s1").await, OwnerPauseKind::Resolvable);
}

// ── read_workflow_run（类型擦除 run 读取） ──────────────────────────

#[tokio::test]
async fn test_read_workflow_run_returns_stored_value() {
    let mgr = make_session_manager();
    insert_session(&mgr, "s1").await;
    let value = run_value(Phase::Verifying, "");
    set_run(&mgr, "s1", value.clone()).await;
    let ops = session_workflow_ops(mgr);
    assert_eq!(ops.read_workflow_run("s1").await, Some(value));
}

#[tokio::test]
async fn test_read_workflow_run_missing_session_is_none() {
    let mgr = make_session_manager();
    let ops = session_workflow_ops(mgr);
    assert_eq!(ops.read_workflow_run("no-such-session").await, None);
}

// ── context append 清理（pub getter 可观测） ────────────────────────

#[tokio::test]
async fn test_remove_workflow_context_append_clears_only_workflow_items() {
    let mgr = make_session_manager();
    insert_session(&mgr, "s1").await;
    {
        let cs = mgr.get_conversation_session("s1").await.unwrap();
        let mut session = cs.write().await;
        session
            .add_system_injection_append("--- WORKFLOW ---\nctx\n--- WORKFLOW END ---".to_string());
        session.add_system_injection_append("unrelated append".to_string());
    }
    let ops = session_workflow_ops(mgr.clone());
    ops.remove_workflow_context_append("s1").await;
    let cs = mgr.get_conversation_session("s1").await.unwrap();
    let session = cs.read().await;
    assert_eq!(session.system_injection_appends(), &["unrelated append"]);
}
