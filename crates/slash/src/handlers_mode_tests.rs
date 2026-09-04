//! Unit tests for PlanModeHandler, ExecuteHandler, and mode parsing.

use std::collections::HashMap;
use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use crate::handlers_mode::{
    parse_plan_path_arg, AutoModeHandler, ExecuteHandler, ModeHandler, PlanModeHandler,
};
use closeclaw_common::slash_router::SlashResult;
use closeclaw_common::SlashSessionQuery;
use closeclaw_config::IdentifierFormat;
use closeclaw_execution::PlanPath;
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
    use closeclaw_session::persistence::ReasoningLevel;

    let gc = closeclaw_gateway::GatewayConfig {
        name: String::new(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        ..Default::default()
    };
    Arc::new(SessionManager::new(
        &gc,
        None,
        None,
        ReasoningLevel::default(),
    ))
}

fn make_plan_handler() -> PlanModeHandler {
    PlanModeHandler::new(
        make_session_manager() as Arc<dyn closeclaw_common::SlashSessionQuery>,
        IdentifierFormat::default(),
    )
}

fn make_auto_handler() -> AutoModeHandler {
    AutoModeHandler::new(make_session_manager() as Arc<dyn closeclaw_common::SlashSessionQuery>)
}

async fn create_test_session(sm: &SessionManager) -> String {
    use closeclaw_gateway::Message;

    let msg = Message {
        id: "mode-test-msg-1".to_string(),
        from: "user-normal".to_string(),
        to: "user-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
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
    assert!(!h.immediate("plan", ""));
}

#[tokio::test]
async fn test_plan_mode_handler_with_args_returns_set_mode() {
    let h = make_plan_handler();
    let ctx = dummy_ctx();
    // Regular args
    match h.handle("实现一个新功能", &ctx).await {
        SlashResult::SetMode {
            mode,
            initial_input,
            reply_message,
            ..
        } => {
            assert_eq!(mode, "plan");
            assert_eq!(initial_input.as_deref(), Some("实现一个新功能"));
            assert_eq!(reply_message.as_deref(), Some("已切换到 Plan 模式"));
        }
        other => panic!("expected SetMode{{mode: \"plan\", ..}}, got {other:?}"),
    }
    // Whitespace-only args still enters plan mode
    match h.handle("  优化性能  ", &ctx).await {
        SlashResult::SetMode { mode, .. } => assert_eq!(mode, "plan"),
        other => panic!("expected SetMode{{mode: \"plan\", ..}}, got {other:?}"),
    }
}

#[tokio::test]
async fn test_plan_mode_handler_with_args_sets_plan_file_path() {
    let sm = make_session_manager();
    let sid = create_test_session(&sm).await;
    let h = PlanModeHandler::new(
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>,
        IdentifierFormat::default(),
    );
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("实现一个新功能", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            ..
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
async fn test_plan_mode_handler_no_args_enters_plan_mode() {
    let sm = make_session_manager_with_storage();
    let sid = create_test_session(&sm).await;
    let h = PlanModeHandler::new(
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>,
        IdentifierFormat::default(),
    );
    let mut ctx = dummy_ctx();
    ctx.session_id = sid.clone();
    // Empty args
    match h.handle("", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            initial_input,
            reply_message,
            ..
        } => {
            assert_eq!(mode, "plan");
            assert!(
                plan_file_path.is_none(),
                "no plan file expected for no-args"
            );
            assert!(
                initial_input.is_none(),
                "initial_input should be None for no-args"
            );
            assert_eq!(reply_message.as_deref(), Some("已切换到 Plan 模式"));
        }
        other => panic!("expected SetMode{{mode: \"plan\", plan_file_path: None}}, got {other:?}"),
    }
    assert!(
        sm.get_plan_state(&sid).await.is_none(),
        "PlanState should NOT be set when /plan is invoked without args"
    );
    // Whitespace-only args — same behavior
    match h.handle("   ", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            ..
        } => {
            assert_eq!(mode, "plan");
            assert!(
                plan_file_path.is_none(),
                "no plan file expected for whitespace-only args"
            );
        }
        other => panic!("expected SetMode{{mode: \"plan\", plan_file_path: None}}, got {other:?}"),
    }
    assert!(
        sm.get_plan_state(&sid).await.is_none(),
        "PlanState should NOT be set when /plan is invoked with whitespace-only args"
    );
}

// ── ModeHandler tests ──────────────────────────────────────────────────────

#[test]
fn test_mode_handler_commands_and_description() {
    let sm = make_session_manager();
    let h = ModeHandler::new(sm as Arc<dyn closeclaw_common::SlashSessionQuery>);
    assert_eq!(h.commands(), &["mode"]);
    assert_eq!(h.description(), "查询或切换会话模式");
}

#[test]
fn test_mode_handler_is_not_immediate_with_args() {
    let sm = make_session_manager();
    let h = ModeHandler::new(sm as Arc<dyn closeclaw_common::SlashSessionQuery>);
    assert!(!h.immediate("mode", "plan"));
    assert!(!h.immediate("mode", "normal"));
    assert!(!h.immediate("mode", "auto"));
}

#[test]
fn test_mode_handler_is_immediate_when_no_args() {
    let sm = make_session_manager();
    let h = ModeHandler::new(sm as Arc<dyn closeclaw_common::SlashSessionQuery>);
    assert!(h.immediate("mode", ""));
    assert!(h.immediate("mode", "   "));
}

#[test]
fn test_mode_handler_is_not_immediate_with_invalid_args() {
    let sm = make_session_manager();
    let h = ModeHandler::new(sm as Arc<dyn closeclaw_common::SlashSessionQuery>);
    assert!(!h.immediate("mode", "invalid"));
}

#[tokio::test]
async fn test_mode_handler_set_plan() {
    let sm = make_session_manager();
    let h = ModeHandler::new(sm as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let ctx = dummy_ctx();
    // Plain args
    match h.handle("plan", &ctx).await {
        SlashResult::SetMode {
            mode,
            initial_input,
            ..
        } => {
            assert_eq!(mode, "plan");
            assert!(
                initial_input.is_none(),
                "initial_input should be None for /mode plan with no task"
            );
        }
        other => panic!("expected SetMode{{mode: \"plan\", ..}}, got {other:?}"),
    }
    // Whitespace-only args
    match h.handle("  plan  ", &ctx).await {
        SlashResult::SetMode {
            mode,
            initial_input,
            ..
        } => {
            assert_eq!(mode, "plan");
            assert!(
                initial_input.is_none(),
                "initial_input should be None for whitespace-only args"
            );
        }
        other => panic!("expected SetMode{{mode: \"plan\", ..}} with whitespace, got {other:?}"),
    }
}

#[tokio::test]
async fn test_mode_handler_auto_returns_set_mode() {
    let sm = make_session_manager_with_storage();
    let sid = create_test_session(&sm).await;
    let h = ModeHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("auto", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            ..
        } => {
            assert_eq!(mode, "auto", "should switch to auto mode from normal mode");
            assert!(plan_file_path.is_none(), "no plan file expected");
        }
        other => {
            panic!("expected SetMode{{mode: \"auto\", plan_file_path: None}}, got {other:?}")
        }
    }
}

#[tokio::test]
async fn test_mode_handler_set_normal() {
    let sm = make_session_manager_with_storage();
    let sid = create_test_session(&sm).await;
    let h = ModeHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
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
    let h = ModeHandler::new(sm as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let ctx = dummy_ctx();
    match h.handle("invalid", &ctx).await {
        SlashResult::Reply(text) => {
            assert_eq!(
                text, "无效模式。可用：normal, plan",
                "should match doc-specified error format"
            );
        }
        other => panic!("expected Reply error, got {other:?}"),
    }
}

#[tokio::test]
async fn test_mode_handler_no_args_queries_current_mode() {
    let sm = make_session_manager();
    let sid = create_test_session(&sm).await;
    let h = ModeHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert_eq!(
                text, "当前模式：Normal",
                "should show current mode with capitalized format"
            );
        }
        other => panic!("expected Reply with current mode, got {other:?}"),
    }
    // Non-existent session
    let h2 = ModeHandler::new(sm as Arc<dyn closeclaw_common::SlashSessionQuery>);
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
    use closeclaw_session::persistence::ReasoningLevel;
    use closeclaw_session::storage::memory::MemoryStorage;

    let gc = closeclaw_gateway::GatewayConfig {
        name: String::new(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        ..Default::default()
    };
    let storage = Arc::new(MemoryStorage::new());
    let sm = SessionManager::new(&gc, Some(storage), None, ReasoningLevel::default());
    Arc::new(sm)
}

async fn create_session_with_plan_mode(sm: &SessionManager) -> String {
    use closeclaw_gateway::Message;

    let msg = Message {
        id: "exec-test-msg-1".to_string(),
        from: "user-plan".to_string(),
        to: "user-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let sid = sm
        .find_or_create("feishu", &msg, None)
        .await
        .expect("session");

    if let Some(conv) = sm.get_conversation_session(&sid).await {
        conv.write().await.set_session_mode(
            closeclaw_common::SessionMode::Plan,
            closeclaw_session::llm_session::mode_transition::ModeChangeSource::Automatic,
        );
    }

    sid
}

#[test]
fn test_execute_handler_commands_and_description() {
    let sm = make_session_manager();
    let h = ExecuteHandler::new(sm as Arc<dyn closeclaw_common::SlashSessionQuery>);
    assert_eq!(h.commands(), &["execute"]);
    assert_eq!(
        h.description(),
        "/execute <plan名称> [附加指令] — 进入 Auto Mode 执行 plan"
    );
}

#[test]
fn test_execute_handler_not_immediate() {
    let sm = make_session_manager();
    let h = ExecuteHandler::new(sm as Arc<dyn closeclaw_common::SlashSessionQuery>);
    assert!(!h.immediate("execute", ""));
}

#[tokio::test]
async fn test_execute_handler_no_session() {
    let sm = make_session_manager();
    let h = ExecuteHandler::new(sm as Arc<dyn closeclaw_common::SlashSessionQuery>);
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
async fn test_execute_non_plan_modes_empty_args_returns_usage_hint() {
    let sm = make_session_manager_with_storage();
    // From Normal mode
    let sid = create_test_session(&sm).await;
    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("请指定要执行的 plan 名称"),
                "should contain usage hint, got: {text}"
            );
        }
        other => panic!("expected Reply with usage hint from Normal, got {other:?}"),
    }
    // From Auto mode
    let sid = create_session_with_auto_mode(&sm).await;
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("请指定要执行的 plan 名称"),
                "should contain usage hint, got: {text}"
            );
        }
        other => panic!("expected Reply with usage hint from Auto, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_handler_no_plan_state() {
    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    // Create a plans dir with a plan file so name resolution works
    let tmp = tempfile::tempdir().unwrap();
    let plans_dir = tmp.path().join("plans");
    std::fs::create_dir_all(&plans_dir).unwrap();
    std::fs::write(plans_dir.join("my-plan.md"), "# Plan\n").unwrap();
    sm.set_workdir(&sid, tmp.path().to_path_buf()).await;

    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    // With valid plan name → resolves and enters auto mode
    match h.handle("my-plan", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            reply_message,
            ..
        } => {
            assert_eq!(mode, "auto");
            assert!(plan_file_path.is_some());
            assert_eq!(reply_message.as_deref(), Some("开始执行"));
        }
        other => panic!("expected SetMode, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_handler_plan_confirmed() {
    use std::fs;

    let tmp = tempfile::tempdir().unwrap();
    let plans_dir = tmp.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    let plan_file = plans_dir.join("test-plan.md");
    fs::write(
        &plan_file,
        "# Test Plan\n\n| 字段 | 值 |\n| 状态 | confirmed |\n",
    )
    .unwrap();

    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    sm.set_workdir(&sid, tmp.path().to_path_buf()).await;

    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("test-plan", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            reply_message,
            ..
        } => {
            assert_eq!(mode, "auto", "should switch to auto mode");
            assert!(plan_file_path.is_some(), "should have plan_file_path");
            let path = plan_file_path.unwrap();
            assert!(
                path.to_string_lossy().ends_with("plans/test-plan.md"),
                "plan_file_path should end with plans/test-plan.md, got: {:?}",
                path
            );
            assert_eq!(reply_message.as_deref(), Some("开始执行"));
        }
        other => panic!("expected SetMode{{mode: \"auto\", ..}}, got {other:?}"),
    }
}

// ── ModeHandler no-args format tests (Step 1.3 — Gap 2) ──────────────────

#[tokio::test]
async fn test_mode_handler_no_args_shows_current_mode() {
    let sm = make_session_manager_with_storage();
    let h = ModeHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    // Plan mode
    let sid = create_session_with_plan_mode(&sm).await;
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => assert_eq!(text, "当前模式：Plan"),
        other => panic!("expected Reply, got {other:?}"),
    }
    // Auto mode
    let sid = create_session_with_auto_mode(&sm).await;
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => assert_eq!(text, "当前模式：Auto"),
        other => panic!("expected Reply, got {other:?}"),
    }
    // Normal mode
    let sid = create_test_session(&sm).await;
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => assert_eq!(text, "当前模式：Normal"),
        other => panic!("expected Reply, got {other:?}"),
    }
}

// NOTE: test_mode_handler_invalid_exact_match removed — it duplicates
// test_mode_handler_invalid_mode which already asserts the exact doc format.

// ── PlanModeHandler transition tests (Step 1.3 — Gap 1 transitions) ─────

// ── /plan --path tests (explicit_path removed from PlanState) ────────────

#[tokio::test]
async fn test_plan_path_no_title_enters_plan_mode() {
    let sm = make_session_manager_with_storage();
    for arg in &["--path interview", "--path standard"] {
        let sid = create_test_session(&sm).await;
        let h = PlanModeHandler::new(
            Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>,
            IdentifierFormat::default(),
        );
        let mut ctx = dummy_ctx();
        ctx.session_id = sid.clone();
        match h.handle(arg, &ctx).await {
            SlashResult::SetMode {
                mode,
                plan_file_path,
                ..
            } => {
                assert_eq!(mode, "plan", "should enter Plan Mode for {arg}");
                assert!(
                    plan_file_path.is_none(),
                    "no plan file for --path without title"
                );
            }
            other => panic!("expected SetMode{{mode: \"plan\"}} for {arg}, got {other:?}"),
        }
    }
}

/// /plan --path should NOT write explicit_path to PlanState.
/// The path is parsed by the handler but no longer stored in PlanState;
/// it belongs in ExecutionState (set by the execution engine).
#[tokio::test]
async fn test_plan_path_does_not_write_explicit_path_to_plan_state() {
    let sm = make_session_manager_with_storage();
    let sid = create_test_session(&sm).await;
    let h = PlanModeHandler::new(
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>,
        IdentifierFormat::default(),
    );
    let mut ctx = dummy_ctx();
    ctx.session_id = sid.clone();
    // /plan --path standard task title
    h.handle("--path standard 实现登录", &ctx).await;
    let plan_state = sm.get_plan_state(&sid).await;
    assert!(
        plan_state.is_some(),
        "plan state should exist after /plan with title"
    );
    let ps = plan_state.unwrap();
    assert_eq!(ps.phase, closeclaw_common::PlanPhase::Research);
    assert!(
        !ps.plan_file_path.is_empty(),
        "plan_file_path should be set"
    );
    // Verify no extra fields — serialize and check
    let json = serde_json::to_value(&ps).unwrap();
    let obj = json.as_object().unwrap();
    assert!(
        !obj.contains_key("explicit_path"),
        "PlanState must NOT have explicit_path field"
    );
    assert!(
        !obj.contains_key("step_selection"),
        "PlanState must NOT have step_selection field"
    );
}

/// /plan --path without title enters plan mode but does NOT create PlanState.
#[tokio::test]
async fn test_plan_path_no_title_no_plan_state() {
    let sm = make_session_manager_with_storage();
    let sid = create_test_session(&sm).await;
    let h = PlanModeHandler::new(
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>,
        IdentifierFormat::default(),
    );
    let mut ctx = dummy_ctx();
    ctx.session_id = sid.clone();
    h.handle("--path interview", &ctx).await;
    let plan_state = sm.get_plan_state(&sid).await;
    assert!(
        plan_state.is_none(),
        "no PlanState should be created for --path without title"
    );
}

// ── AutoModeHandler tests ─────────────────────────────────────────────────

#[test]
fn test_auto_mode_handler_commands_and_description() {
    let h = make_auto_handler();
    assert_eq!(h.commands(), &["auto"]);
    assert_eq!(h.description(), "直接进入 Auto Mode");
}

#[test]
fn test_auto_mode_handler_not_immediate() {
    let h = make_auto_handler();
    assert!(!h.immediate("auto", ""));
}

#[tokio::test]
async fn test_auto_no_args_enters_auto_mode() {
    let sm = make_session_manager_with_storage();
    let sid = create_test_session(&sm).await;
    let h = AutoModeHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            ..
        } => {
            assert_eq!(mode, "auto");
            assert!(plan_file_path.is_none(), "no plan file expected");
        }
        other => panic!("expected SetMode{{mode: \"auto\", ..}}, got {other:?}"),
    }
}

#[tokio::test]
async fn test_auto_already_in_auto_mode() {
    let sm = make_session_manager_with_storage();
    let sid = create_session_with_auto_mode(&sm).await;
    let h = AutoModeHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("已在 Auto Mode"), "got: {text}");
        }
        other => panic!("expected Reply already in Auto Mode, got {other:?}"),
    }
}

// ── AutoModeHandler helper (for Plan transition tests) ───────────────────

async fn create_session_with_auto_mode(sm: &SessionManager) -> String {
    use closeclaw_gateway::Message;

    let msg = Message {
        id: "auto-mode-test-msg".to_string(),
        from: "user-auto".to_string(),
        to: "user-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let sid = sm
        .find_or_create("feishu", &msg, None)
        .await
        .expect("session");

    if let Some(conv) = sm.get_conversation_session(&sid).await {
        conv.write().await.set_session_mode(
            closeclaw_common::SessionMode::Auto,
            closeclaw_session::llm_session::mode_transition::ModeChangeSource::Automatic,
        );
    }

    sid
}

#[tokio::test]
async fn test_mode_handler_normal_from_all_modes() {
    let sm = make_session_manager_with_storage();
    let h = ModeHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    // From Normal Mode
    let sid = create_test_session(&sm).await;
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("normal", &ctx).await {
        SlashResult::SetMode { mode, .. } => assert_eq!(mode, "normal"),
        other => panic!("expected SetMode for /mode normal from Normal, got {other:?}"),
    }
    // From Plan Mode
    let sid = create_session_with_plan_mode(&sm).await;
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("normal", &ctx).await {
        SlashResult::SetMode { mode, .. } => assert_eq!(mode, "normal"),
        other => panic!("expected SetMode for /mode normal from Plan, got {other:?}"),
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

// ── /mode plan delegation test (Step 1.6) ──────────────────────────────

#[tokio::test]
async fn test_mode_delegation_equivalence() {
    let sm = make_session_manager_with_storage();
    let plan_h = Arc::new(PlanModeHandler::new(
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>,
        IdentifierFormat::default(),
    ));
    let auto_h = Arc::new(AutoModeHandler::new(
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>
    ));
    let h = ModeHandler::with_handlers(
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>,
        plan_h,
        auto_h,
    );
    // /mode plan 任务 → equivalent to /plan 任务
    let mut ctx = dummy_ctx();
    let sid = create_test_session(&sm).await;
    ctx.session_id = sid;
    match h.handle("plan 任务", &ctx).await {
        SlashResult::SetMode {
            mode,
            initial_input,
            plan_file_path,
            reply_message,
            ..
        } => {
            assert_eq!(mode, "plan");
            assert_eq!(initial_input.as_deref(), Some("任务"));
            assert!(plan_file_path.is_some(), "should create plan file");
            assert_eq!(reply_message.as_deref(), Some("已切换到 Plan 模式"));
        }
        other => panic!("expected SetMode for /mode plan 任务, got {other:?}"),
    }
    // /mode auto → equivalent to /auto
    let sid = create_test_session(&sm).await;
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("auto", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            reply_message,
            ..
        } => {
            assert_eq!(mode, "auto");
            assert!(plan_file_path.is_none());
            assert_eq!(reply_message.as_deref(), Some("已切换到 Auto 模式"));
        }
        other => panic!("expected SetMode for /mode auto, got {other:?}"),
    }
}

// ── ExecuteHandler error path tests (Step 1.4) ───────────────────────────
// Split into handlers_execute_tests.rs to keep this file under
// the 1000-line limit.
#[path = "handlers_execute_tests.rs"]
mod handlers_execute_tests;
