//! Unit tests for PlanModeHandler and ModeHandler.

use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use crate::handlers_mode::{
    parse_plan_status_from_file, ExecuteHandler, ModeHandler, PlanModeHandler,
};
use closeclaw_common::plan_state::PlanStatus;
use closeclaw_common::slash_router::SlashResult;
use closeclaw_gateway::session_manager::SessionManager;

// ── Helpers ────────────────────────────────────────────────────────────────

fn dummy_ctx() -> SlashContext {
    SlashContext {
        command: String::new(),
        sender_id: "test_sender".to_owned(),
        session_id: "test_session".to_owned(),
        channel: "test_channel".to_owned(),
    }
}

fn make_session_manager() -> Arc<SessionManager> {
    use closeclaw_gateway::DmScope;
    use closeclaw_session::bootstrap::loader::BootstrapMode;
    use closeclaw_session::persistence::ReasoningLevel;

    let gc = closeclaw_gateway::GatewayConfig {
        name: String::new(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        dm_scope: DmScope::default(),
        ..Default::default()
    };
    Arc::new(SessionManager::new(
        &gc,
        None, // storage
        None, // workspace_dir
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ))
}

fn make_plan_handler() -> PlanModeHandler {
    PlanModeHandler::new(make_session_manager())
}

async fn create_test_session(sm: &SessionManager) -> String {
    use closeclaw_gateway::Message;

    let msg = Message {
        id: "mode-test-msg-1".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
    };
    sm.find_or_create("feishu", &msg, None)
        .await
        .expect("session")
}

// ── PlanModeHandler tests ──────────────────────────────────────────────────

#[test]
fn test_plan_mode_handler_commands_and_description() {
    let h = make_plan_handler();
    assert_eq!(h.commands(), &["plan"]);
    assert_eq!(h.description(), "进入 Plan Mode");
}

#[test]
fn test_plan_mode_handler_not_immediate() {
    let h = make_plan_handler();
    assert!(!h.immediate("plan"));
}

#[tokio::test]
async fn test_plan_mode_handler_with_args_returns_set_mode() {
    let h = make_plan_handler();
    let ctx = dummy_ctx();
    match h.handle("实现一个新功能", &ctx).await {
        SlashResult::SetMode { mode, .. } => assert_eq!(mode, "plan"),
        other => panic!("expected SetMode{{mode: \"plan\", ..}}, got {other:?}"),
    }
}

#[tokio::test]
async fn test_plan_mode_handler_with_args_sets_plan_file_path() {
    let sm = make_session_manager();
    let sid = create_test_session(&sm).await;
    let h = PlanModeHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("实现一个新功能", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
        } => {
            assert_eq!(mode, "plan");
            assert!(
                plan_file_path.is_some(),
                "plan_file_path should be Some when args are provided"
            );
            let path = plan_file_path.unwrap();
            assert!(
                path.to_string_lossy().contains("plans"),
                "plan file path should be under plans/ directory, got: {:?}",
                path
            );
            assert!(path.exists(), "plan file should exist on disk");
        }
        other => {
            panic!("expected SetMode{{mode: \"plan\", plan_file_path: Some(..)}}, got {other:?}")
        }
    }
}

#[tokio::test]
async fn test_plan_mode_handler_with_whitespace_args_returns_set_mode() {
    let h = make_plan_handler();
    let ctx = dummy_ctx();
    match h.handle("  优化性能  ", &ctx).await {
        SlashResult::SetMode { mode, .. } => assert_eq!(mode, "plan"),
        other => panic!("expected SetMode{{mode: \"plan\", ..}}, got {other:?}"),
    }
}

#[tokio::test]
async fn test_plan_mode_handler_no_args_returns_usage() {
    let h = make_plan_handler();
    let ctx = dummy_ctx();
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("用法"),
                "should contain usage hint, got: {text}"
            );
            assert!(text.contains("/plan"), "should mention /plan, got: {text}");
        }
        other => panic!("expected Reply with usage, got {other:?}"),
    }
}

#[tokio::test]
async fn test_plan_mode_handler_whitespace_only_args_returns_usage() {
    let h = make_plan_handler();
    let ctx = dummy_ctx();
    match h.handle("   ", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("用法"),
                "should contain usage hint, got: {text}"
            );
        }
        other => panic!("expected Reply with usage, got {other:?}"),
    }
}

// ── ModeHandler tests ──────────────────────────────────────────────────────

#[test]
fn test_mode_handler_commands_and_description() {
    let sm = make_session_manager();
    let h = ModeHandler::new(sm);
    assert_eq!(h.commands(), &["mode"]);
    assert_eq!(h.description(), "查询或切换会话模式");
}

#[test]
fn test_mode_handler_is_immediate() {
    let sm = make_session_manager();
    let h = ModeHandler::new(sm);
    assert!(h.immediate("mode"));
}

#[tokio::test]
async fn test_mode_handler_set_plan() {
    let sm = make_session_manager();
    let h = ModeHandler::new(sm);
    let ctx = dummy_ctx();
    match h.handle("plan", &ctx).await {
        SlashResult::SetMode { mode, .. } => assert_eq!(mode, "plan"),
        other => panic!("expected SetMode{{mode: \"plan\", ..}}, got {other:?}"),
    }
}

#[tokio::test]
async fn test_mode_handler_set_auto() {
    let sm = make_session_manager();
    let h = ModeHandler::new(sm);
    let ctx = dummy_ctx();
    match h.handle("auto", &ctx).await {
        SlashResult::SetMode { mode, .. } => assert_eq!(mode, "auto"),
        other => panic!("expected SetMode{{mode: \"auto\", ..}}, got {other:?}"),
    }
}

#[tokio::test]
async fn test_mode_handler_set_normal() {
    let sm = make_session_manager();
    let h = ModeHandler::new(sm);
    let ctx = dummy_ctx();
    match h.handle("normal", &ctx).await {
        SlashResult::SetMode { mode, .. } => assert_eq!(mode, "normal"),
        other => panic!("expected SetMode{{mode: \"normal\", ..}}, got {other:?}"),
    }
}

#[tokio::test]
async fn test_mode_handler_invalid_mode() {
    let sm = make_session_manager();
    let h = ModeHandler::new(sm);
    let ctx = dummy_ctx();
    match h.handle("invalid", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("无效"),
                "should indicate invalid, got: {text}"
            );
            assert!(
                text.contains("normal"),
                "should list valid options, got: {text}"
            );
            assert!(
                text.contains("plan"),
                "should list valid options, got: {text}"
            );
            assert!(
                text.contains("auto"),
                "should list valid options, got: {text}"
            );
        }
        other => panic!("expected Reply error, got {other:?}"),
    }
}

#[tokio::test]
async fn test_mode_handler_with_args_whitespace() {
    let sm = make_session_manager();
    let h = ModeHandler::new(sm);
    let ctx = dummy_ctx();
    match h.handle("  plan  ", &ctx).await {
        SlashResult::SetMode { mode, .. } => assert_eq!(mode, "plan"),
        other => panic!("expected SetMode{{mode: \"plan\", ..}} with whitespace, got {other:?}"),
    }
}

#[tokio::test]
async fn test_mode_handler_no_args_queries_current_mode() {
    let sm = make_session_manager();
    let sid = create_test_session(&sm).await;
    let h = ModeHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("当前会话模式"),
                "should report current mode, got: {text}"
            );
            // Default mode is Normal
            assert!(
                text.contains("normal"),
                "default should be normal, got: {text}"
            );
        }
        other => panic!("expected Reply with current mode, got {other:?}"),
    }
}

#[tokio::test]
async fn test_mode_handler_no_args_no_session() {
    let sm = make_session_manager();
    let h = ModeHandler::new(sm);
    let ctx = SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: "nonexistent".to_owned(),
        channel: "c".to_owned(),
    };
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("当前会话未激活"),
                "should indicate inactive session, got: {text}"
            );
        }
        other => panic!("expected Reply with no-session, got {other:?}"),
    }
}

// ── ExecuteHandler tests ─────────────────────────────────────────────────

fn make_session_manager_with_storage() -> Arc<SessionManager> {
    use closeclaw_gateway::DmScope;
    use closeclaw_session::bootstrap::loader::BootstrapMode;
    use closeclaw_session::persistence::ReasoningLevel;
    use closeclaw_session::storage::memory::MemoryStorage;

    let gc = closeclaw_gateway::GatewayConfig {
        name: String::new(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        dm_scope: DmScope::default(),
        ..Default::default()
    };
    let storage = Arc::new(MemoryStorage::new());
    let sm = SessionManager::new(
        &gc,
        Some(storage),
        None, // workspace_dir
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );
    Arc::new(sm)
}

async fn create_session_with_plan_mode(sm: &SessionManager) -> String {
    use closeclaw_gateway::Message;

    let msg = Message {
        id: "exec-test-msg-1".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
    };
    let sid = sm
        .find_or_create("feishu", &msg, None)
        .await
        .expect("session");

    // Set session mode to Plan
    if let Some(conv) = sm.get_conversation_session(&sid).await {
        conv.write()
            .await
            .set_session_mode(closeclaw_common::SessionMode::Plan);
    }

    sid
}

async fn save_plan_state(sm: &SessionManager, session_id: &str, plan_file_path: &str) {
    use closeclaw_common::{PlanPhase, PlanState};

    sm.set_plan_state(
        session_id,
        PlanState {
            phase: PlanPhase::FinalPlan,
            plan_file_path: plan_file_path.to_string(),
            ..PlanState::new()
        },
    )
    .await;
}

async fn save_plan_state_with_status(
    sm: &SessionManager,
    session_id: &str,
    plan_file_path: &str,
    status: PlanStatus,
) {
    use closeclaw_common::{PlanPhase, PlanState};

    sm.set_plan_state(
        session_id,
        PlanState {
            phase: PlanPhase::FinalPlan,
            status,
            plan_file_path: plan_file_path.to_string(),
            ..PlanState::new()
        },
    )
    .await;
}

#[test]
fn test_execute_handler_commands_and_description() {
    let sm = make_session_manager();
    let h = ExecuteHandler::new(sm);
    assert_eq!(h.commands(), &["execute"]);
    assert_eq!(h.description(), "从 Plan Mode 进入 Auto Mode 执行");
}

#[test]
fn test_execute_handler_not_immediate() {
    let sm = make_session_manager();
    let h = ExecuteHandler::new(sm);
    assert!(!h.immediate("execute"));
}

#[tokio::test]
async fn test_execute_handler_no_session() {
    let sm = make_session_manager();
    let h = ExecuteHandler::new(sm);
    let ctx = SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: "nonexistent".to_owned(),
        channel: "c".to_owned(),
    };
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("当前会话未激活"), "got: {text}");
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_handler_not_in_plan_mode() {
    let sm = make_session_manager_with_storage();
    let sid = create_test_session(&sm).await;
    let h = ExecuteHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("Plan Mode"),
                "should mention Plan Mode, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_handler_no_plan_state() {
    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    let h = ExecuteHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("没有活跃的 plan"),
                "should mention no active plan, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_handler_plan_not_confirmed() {
    use std::fs;

    let tmp = tempfile::tempdir().unwrap();
    let plan_file = tmp.path().join("test-plan.md");
    fs::write(
        &plan_file,
        "# Test Plan\n\n| 字段 | 值 |\n| 状态 | draft |\n",
    )
    .unwrap();

    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    save_plan_state(&sm, &sid, plan_file.to_str().unwrap()).await;

    let h = ExecuteHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("尚未通过审批"),
                "should mention not approved, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_handler_plan_confirmed() {
    use std::fs;

    let tmp = tempfile::tempdir().unwrap();
    let plan_file = tmp.path().join("test-plan.md");
    fs::write(
        &plan_file,
        "# Test Plan\n\n| 字段 | 值 |\n| 状态 | confirmed |\n",
    )
    .unwrap();

    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    save_plan_state(&sm, &sid, plan_file.to_str().unwrap()).await;

    let h = ExecuteHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
        } => {
            assert_eq!(mode, "auto", "should switch to auto mode");
            assert!(plan_file_path.is_some(), "should have plan_file_path");
            assert_eq!(
                plan_file_path.unwrap(),
                plan_file,
                "plan_file_path should match"
            );
        }
        other => panic!("expected SetMode{{mode: \"auto\", ..}}, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_handler_plan_confirmed_updates_to_executing() {
    use std::fs;

    let tmp = tempfile::tempdir().unwrap();
    let plan_file = tmp.path().join("test-plan.md");
    fs::write(
        &plan_file,
        "# Test Plan\n\n| 字段 | 值 |\n| 状态 | confirmed |\n",
    )
    .unwrap();

    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    save_plan_state(&sm, &sid, plan_file.to_str().unwrap()).await;

    let h = ExecuteHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    let _ = h.handle("", &ctx).await;

    // Verify plan file status was updated to executing
    let content = fs::read_to_string(&plan_file).unwrap();
    assert!(
        content.contains("| 状态 | executing |"),
        "plan file should be updated to executing status, got: {content}"
    );
}

#[test]
fn test_update_plan_status_direct() {
    use std::fs;

    let tmp = tempfile::tempdir().unwrap();
    let plan_file = tmp.path().join("test-plan.md");
    fs::write(
        &plan_file,
        "# Test Plan\n\n| 字段 | 值 |\n| 状态 | draft |\n",
    )
    .unwrap();

    let result = closeclaw_session::plan_file::update_plan_status(
        plan_file.to_str().unwrap(),
        &closeclaw_common::PlanStatus::Confirmed,
    );
    assert!(
        result.is_ok(),
        "update_plan_status should succeed: {:?}",
        result
    );

    let content = fs::read_to_string(&plan_file).unwrap();
    assert!(
        content.contains("| 状态 | confirmed |"),
        "plan file should show confirmed, got: {content}"
    );
}

#[tokio::test]
async fn test_execute_handler_plan_state_with_confirmed_status() {
    use std::fs;

    let tmp = tempfile::tempdir().unwrap();
    let plan_file = tmp.path().join("test-plan.md");
    fs::write(
        &plan_file,
        "# Test Plan\n\n| 字段 | 值 |\n| 状态 | draft |\n",
    )
    .unwrap();

    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    // Set in-memory status to Confirmed (file still says draft)
    save_plan_state_with_status(
        &sm,
        &sid,
        plan_file.to_str().unwrap(),
        PlanStatus::Confirmed,
    )
    .await;

    let h = ExecuteHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::SetMode { mode, .. } => {
            assert_eq!(
                mode, "auto",
                "should switch to auto mode based on in-memory status"
            );
        }
        other => panic!("expected SetMode, got {other:?}"),
    }
}

// ── parse_plan_status_from_file tests ─────────────────────────────────────

#[test]
fn test_parse_plan_status_draft() {
    let content = "| 状态 | draft |";
    assert_eq!(
        parse_plan_status_from_file(content),
        Some(PlanStatus::Draft)
    );
}

#[test]
fn test_parse_plan_status_confirmed() {
    let content = "| 状态 | confirmed |";
    assert_eq!(
        parse_plan_status_from_file(content),
        Some(PlanStatus::Confirmed)
    );
}

#[test]
fn test_parse_plan_status_executing() {
    let content = "| 状态 | executing |";
    assert_eq!(
        parse_plan_status_from_file(content),
        Some(PlanStatus::Executing)
    );
}

#[test]
fn test_parse_plan_status_paused() {
    let content = "| 状态 | paused |";
    assert_eq!(
        parse_plan_status_from_file(content),
        Some(PlanStatus::Paused)
    );
}

#[test]
fn test_parse_plan_status_completed() {
    let content = "| 状态 | completed |";
    assert_eq!(
        parse_plan_status_from_file(content),
        Some(PlanStatus::Completed)
    );
}

#[test]
fn test_parse_plan_status_not_found() {
    let content = "| 字段 | 值 |\n| 状态 |";
    assert_eq!(parse_plan_status_from_file(content), None);
}

#[test]
fn test_parse_plan_status_unknown_value() {
    let content = "| 状态 | unknown |";
    assert_eq!(parse_plan_status_from_file(content), None);
}

#[test]
fn test_parse_plan_status_in_full_plan() {
    let content = "# Test Plan\n\n| 字段 | 值 |\n| 状态 | confirmed |\n| 创建时间 | 2025-01-01 |\n";
    assert_eq!(
        parse_plan_status_from_file(content),
        Some(PlanStatus::Confirmed)
    );
}

#[tokio::test]
async fn test_execute_handler_plan_file_not_readable() {
    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    save_plan_state(&sm, &sid, "/nonexistent/path/plan.md").await;

    let h = ExecuteHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("无法读取 plan 文件"),
                "should mention file read error, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}
