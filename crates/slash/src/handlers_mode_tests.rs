//! Unit tests for PlanModeHandler, ExecuteHandler, and mode parsing.

use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use crate::handlers_mode::{
    parse_plan_path_arg, parse_plan_status_from_file, ExecuteHandler, ModeHandler, PlanModeHandler,
};
use closeclaw_common::plan_state::{PlanPath, PlanStatus};
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
        None,
        None,
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
async fn test_mode_handler_auto_blocked() {
    let sm = make_session_manager();
    let h = ModeHandler::new(sm);
    let ctx = dummy_ctx();
    match h.handle("auto", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("不能直接"),
                "should explain auto cannot be entered directly, got: {text}"
            );
            assert!(
                text.contains("/execute"),
                "should mention /execute as the correct path, got: {text}"
            );
        }
        other => panic!("expected Reply blocking auto mode, got {other:?}"),
    }
}

#[tokio::test]
async fn test_mode_handler_set_normal() {
    let sm = make_session_manager_with_storage();
    let sid = create_test_session(&sm).await;
    let h = ModeHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
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
            assert!(text.contains("当前会话模式"), "got: {text}");
            assert!(
                text.contains("normal"),
                "default should be normal, got: {text}"
            );
        }
        other => panic!("expected Reply with current mode, got {other:?}"),
    }
    // Non-existent session
    let h2 = ModeHandler::new(sm);
    let ctx2 = SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: "nonexistent".to_owned(),
        channel: "c".to_owned(),
    };
    match h2.handle("", &ctx2).await {
        SlashResult::Reply(text) => assert!(text.contains("当前会话未激活"), "got: {text}"),
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── parse_plan_path_arg tests ──────────────────────────────────────────────

#[test]
fn test_parse_plan_path_all_cases() {
    // Valid path with title
    assert_eq!(
        parse_plan_path_arg("--path standard 实现登录功能"),
        (Some(PlanPath::Standard), "实现登录功能")
    );
    assert_eq!(
        parse_plan_path_arg("--path interview 优化性能"),
        (Some(PlanPath::Interview), "优化性能")
    );
    // No --path
    assert_eq!(parse_plan_path_arg("实现新功能"), (None, "实现新功能"));
    // Path only (no title)
    assert_eq!(
        parse_plan_path_arg("--path standard"),
        (Some(PlanPath::Standard), "")
    );
    assert_eq!(
        parse_plan_path_arg("--path interview"),
        (Some(PlanPath::Interview), "")
    );
    // Invalid path value
    assert_eq!(
        parse_plan_path_arg("--path invalid 任务标题"),
        (None, "任务标题")
    );
    // Whitespace handling
    assert_eq!(parse_plan_path_arg("--path  任务标题"), (None, "任务标题"));
    assert_eq!(
        parse_plan_path_arg("  --path standard  优化性能  "),
        (Some(PlanPath::Standard), "优化性能")
    );
    // Chinese title
    assert_eq!(
        parse_plan_path_arg("--path standard 修复登录页面的样式问题"),
        (Some(PlanPath::Standard), "修复登录页面的样式问题")
    );
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
        None,
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
                text.contains("当前 plan 未就绪"),
                "should mention plan not ready, got: {text}"
            );
            assert!(
                text.contains("plan_approval"),
                "should mention plan_approval tool, got: {text}"
            );
            assert!(
                text.contains("暂停"),
                "should mention pause as an alternative, got: {text}"
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
fn test_parse_plan_status_all_variants() {
    assert_eq!(
        parse_plan_status_from_file("| 状态 | draft |"),
        Some(PlanStatus::Draft)
    );
    assert_eq!(
        parse_plan_status_from_file("| 状态 | confirmed |"),
        Some(PlanStatus::Confirmed)
    );
    assert_eq!(
        parse_plan_status_from_file("| 状态 | executing |"),
        Some(PlanStatus::Executing)
    );
    assert_eq!(
        parse_plan_status_from_file("| 状态 | paused |"),
        Some(PlanStatus::Paused)
    );
    assert_eq!(
        parse_plan_status_from_file("| 状态 | completed |"),
        Some(PlanStatus::Completed)
    );
    // Edge cases
    assert_eq!(parse_plan_status_from_file("| 字段 | 值 |\n| 状态 |"), None);
    assert_eq!(parse_plan_status_from_file("| 状态 | unknown |"), None);
    let full_plan =
        "# Test Plan\n\n| 字段 | 值 |\n| 状态 | confirmed |\n| 创建时间 | 2025-01-01 |\n";
    assert_eq!(
        parse_plan_status_from_file(full_plan),
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

// ── ExecuteHandler file fallback tests ───────────────────────────────────

#[tokio::test]
async fn test_execute_handler_falls_back_to_file_status() {
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
        SlashResult::SetMode { mode, .. } => {
            assert_eq!(
                mode, "auto",
                "should fall back to file status and switch to auto"
            );
        }
        other => panic!("expected SetMode, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_handler_uses_in_memory_status_when_non_default() {
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
            assert_eq!(mode, "auto", "should use in-memory Confirmed status");
        }
        other => panic!("expected SetMode, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_handler_file_no_status_line() {
    use std::fs;

    let tmp = tempfile::tempdir().unwrap();
    let plan_file = tmp.path().join("test-plan.md");
    fs::write(&plan_file, "# Test Plan\n\nNo status field here.\n").unwrap();

    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    save_plan_state(&sm, &sid, plan_file.to_str().unwrap()).await;

    let h = ExecuteHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("未找到有效的状态字段"),
                "should mention missing status field, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_handler_empty_plan_file_path() {
    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    sm.set_plan_state(
        &sid,
        closeclaw_common::PlanState {
            phase: closeclaw_common::PlanPhase::FinalPlan,
            status: PlanStatus::Confirmed,
            plan_file_path: String::new(),
            ..closeclaw_common::PlanState::new()
        },
    )
    .await;

    let h = ExecuteHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("没有关联的 plan 文件"),
                "should mention no plan file, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── ExecuteHandler Paused resume tests (Step 1.2 — Gap 2) ────────────────

#[tokio::test]
async fn test_execute_handler_plan_paused_resumes_and_updates_file() {
    use std::fs;

    let tmp = tempfile::tempdir().unwrap();
    let plan_file = tmp.path().join("test-plan.md");
    fs::write(
        &plan_file,
        "# Test Plan\n\n| 字段 | 值 |\n| 状态 | paused |\n",
    )
    .unwrap();

    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    save_plan_state_with_status(&sm, &sid, plan_file.to_str().unwrap(), PlanStatus::Paused).await;

    let h = ExecuteHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
        } => {
            assert_eq!(mode, "auto", "should switch to auto mode on resume");
            assert!(plan_file_path.is_some(), "should have plan_file_path");
            assert_eq!(
                plan_file_path.unwrap(),
                plan_file,
                "plan_file_path should match"
            );
        }
        other => panic!("expected SetMode for Paused resume, got {other:?}"),
    }
    // Verify plan file updated to executing
    let content = fs::read_to_string(&plan_file).unwrap();
    assert!(
        content.contains("| 状态 | executing |"),
        "plan file should be updated to executing status, got: {content}"
    );
}

#[tokio::test]
async fn test_execute_handler_plan_paused_falls_back_to_file_status() {
    use std::fs;

    let tmp = tempfile::tempdir().unwrap();
    let plan_file = tmp.path().join("test-plan.md");
    fs::write(
        &plan_file,
        "# Test Plan\n\n| 字段 | 值 |\n| 状态 | paused |\n",
    )
    .unwrap();

    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    // Save with Draft status (default) so it falls back to file
    save_plan_state(&sm, &sid, plan_file.to_str().unwrap()).await;

    let h = ExecuteHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::SetMode { mode, .. } => {
            assert_eq!(
                mode, "auto",
                "should fall back to file Paused status and resume"
            );
        }
        other => panic!("expected SetMode for Paused file fallback, got {other:?}"),
    }
}

// ── ModeHandler approval gate tests (Step 1.5 — Gap 2) ─────────────────

async fn create_session_with_auto_mode(sm: &SessionManager) -> String {
    use closeclaw_gateway::Message;

    let msg = Message {
        id: "auto-mode-test-msg".to_string(),
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

    if let Some(conv) = sm.get_conversation_session(&sid).await {
        conv.write()
            .await
            .set_session_mode(closeclaw_common::SessionMode::Auto);
    }

    sid
}

#[tokio::test]
async fn test_mode_handler_normal_from_plan_mode_rejected() {
    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    let h = ModeHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("normal", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("Plan Mode"),
                "should mention Plan Mode, got: {text}"
            );
            assert!(
                text.contains("plan_approval"),
                "should guide user to plan_approval tool, got: {text}"
            );
        }
        other => {
            panic!("expected Reply rejecting /mode normal from Plan Mode, got {other:?}")
        }
    }
}

#[tokio::test]
async fn test_mode_handler_normal_from_non_plan_modes_allowed() {
    let sm = make_session_manager_with_storage();
    // From Normal Mode
    let sid = create_test_session(&sm).await;
    let h = ModeHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("normal", &ctx).await {
        SlashResult::SetMode { mode, .. } => assert_eq!(mode, "normal"),
        other => panic!("expected SetMode for /mode normal from Normal, got {other:?}"),
    }
    // From Auto Mode
    let sid = create_session_with_auto_mode(&sm).await;
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("normal", &ctx).await {
        SlashResult::SetMode { mode, .. } => assert_eq!(mode, "normal"),
        other => panic!("expected SetMode for /mode normal from Auto, got {other:?}"),
    }
}

#[tokio::test]
async fn test_mode_handler_plan_from_plan_mode_allowed() {
    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    let h = ModeHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("plan", &ctx).await {
        SlashResult::SetMode { mode, .. } => assert_eq!(mode, "plan"),
        other => panic!("expected SetMode for /mode plan from Plan Mode, got {other:?}"),
    }
}
