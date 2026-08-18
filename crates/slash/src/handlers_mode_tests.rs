//! Unit tests for PlanModeHandler, ExecuteHandler, and mode parsing.

use std::collections::HashMap;
use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use crate::handlers_mode::{
    parse_plan_path_arg, AutoModeHandler, ExecuteHandler, ModeHandler, PlanModeHandler,
};
use closeclaw_common::plan_state::PlanPath;
use closeclaw_common::slash_router::SlashResult;
use closeclaw_common::SlashSessionQuery;
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
    PlanModeHandler::new(make_session_manager() as Arc<dyn closeclaw_common::SlashSessionQuery>)
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
    assert!(!h.immediate("plan"));
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
    let h = PlanModeHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
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
    let h = PlanModeHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
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
fn test_mode_handler_is_not_immediate() {
    let sm = make_session_manager();
    let h = ModeHandler::new(sm as Arc<dyn closeclaw_common::SlashSessionQuery>);
    assert!(!h.immediate("mode"));
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
                text, "无效模式。可用：normal, plan, auto",
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
                text, "当前模式：normal",
                "should show current mode with doc format"
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
        platform: None,
        dsl_result: None,
        content_blocks: None,
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
    assert!(!h.immediate("execute"));
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
async fn test_execute_non_plan_modes_enters_auto() {
    let sm = make_session_manager_with_storage();
    // From Normal mode
    let sid = create_test_session(&sm).await;
    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            reply_message,
            ..
        } => {
            assert_eq!(mode, "auto");
            assert!(plan_file_path.is_none());
            assert_eq!(reply_message.as_deref(), Some("开始执行"));
        }
        other => panic!("expected SetMode from Normal, got {other:?}"),
    }
    // From Auto mode
    let sid = create_session_with_auto_mode(&sm).await;
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            reply_message,
            ..
        } => {
            assert_eq!(mode, "auto");
            assert!(plan_file_path.is_none());
            assert_eq!(reply_message.as_deref(), Some("开始执行"));
        }
        other => panic!("expected SetMode from Auto, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_handler_no_plan_state() {
    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid.clone();
    // No plan state
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("没有活跃的 plan"), "got: {text}");
        }
        other => panic!("expected Reply, got {other:?}"),
    }
    // Plan state with empty file path
    sm.set_plan_state(
        &sid,
        closeclaw_common::PlanState {
            phase: closeclaw_common::PlanPhase::FinalPlan,
            plan_file_path: String::new(),
            ..closeclaw_common::PlanState::new()
        },
    )
    .await;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("没有关联的 plan 文件"), "got: {text}");
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

    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            reply_message,
            ..
        } => {
            assert_eq!(mode, "auto", "should switch to auto mode");
            assert!(plan_file_path.is_some(), "should have plan_file_path");
            assert_eq!(
                plan_file_path.unwrap(),
                plan_file,
                "plan_file_path should match"
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
        SlashResult::Reply(text) => assert_eq!(text, "当前模式：plan"),
        other => panic!("expected Reply, got {other:?}"),
    }
    // Auto mode
    let sid = create_session_with_auto_mode(&sm).await;
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => assert_eq!(text, "当前模式：auto"),
        other => panic!("expected Reply, got {other:?}"),
    }
    // Normal mode
    let sid = create_test_session(&sm).await;
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => assert_eq!(text, "当前模式：normal"),
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
        let h =
            PlanModeHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
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
    let h = PlanModeHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
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
    let h = PlanModeHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
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
    assert!(!h.immediate("auto"));
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
        platform: None,
        dsl_result: None,
        content_blocks: None,
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
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>
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

// ── parse_execute_args tests ────────────────────────────────────────────

use crate::handlers_mode::parse_execute_args;

#[test]
fn test_parse_execute_args_all_cases() {
    // Empty / whitespace-only → no name, no instruction
    let (n, i) = parse_execute_args("");
    assert!(n.is_none() && i.is_none());
    let (n, i) = parse_execute_args("   ");
    assert!(n.is_none() && i.is_none());
    // Name only
    let (n, i) = parse_execute_args("foo");
    assert_eq!(n.as_deref(), Some("foo"));
    assert!(i.is_none());
    // Name + instruction
    let (n, i) = parse_execute_args("foo bar baz");
    assert_eq!(n.as_deref(), Some("foo"));
    assert_eq!(i.as_deref(), Some("bar baz"));
    // Extra whitespace trimmed around name
    let (n, i) = parse_execute_args("  foo  bar baz  ");
    assert_eq!(n.as_deref(), Some("foo"));
    assert_eq!(i.as_deref(), Some("bar baz"));
    // Whitespace-only instruction → None
    let (n, i) = parse_execute_args("foo   ");
    assert_eq!(n.as_deref(), Some("foo"));
    assert!(i.is_none());
    // Name with .md suffix works
    let (n, i) = parse_execute_args("plan.md instruction");
    assert_eq!(n.as_deref(), Some("plan.md"));
    assert_eq!(i.as_deref(), Some("instruction"));
    // Chinese name + instruction
    let (n, i) = parse_execute_args("修复登录 请优先处理");
    assert_eq!(n.as_deref(), Some("修复登录"));
    assert_eq!(i.as_deref(), Some("请优先处理"));
}

// ── ExecuteHandler with name/instruction tests ─────────────────────────

#[tokio::test]
async fn test_execute_plan_mode_with_name_resolves_plan() {
    use std::fs;
    let tmp = tempfile::tempdir().unwrap();
    let plans_dir = tmp.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    let plan_file = plans_dir.join("my-plan.md");
    fs::write(&plan_file, "# My Plan\n").unwrap();
    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    sm.set_workdir(&sid, tmp.path().to_path_buf()).await;
    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    // name only
    match h.handle("my-plan", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            initial_input,
            reply_message,
            ..
        } => {
            assert_eq!(mode, "auto");
            let path = plan_file_path.unwrap();
            assert!(path.to_string_lossy().ends_with("my-plan.md"));
            assert!(initial_input.is_none());
            assert_eq!(reply_message.as_deref(), Some("开始执行"));
        }
        other => panic!("expected SetMode{{mode: \"auto\", ..}}, got {other:?}"),
    }
    // name + instruction
    match h.handle("my-plan 请优先处理", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            initial_input,
            ..
        } => {
            assert_eq!(mode, "auto");
            assert!(plan_file_path.is_some());
            assert_eq!(initial_input.as_deref(), Some("请优先处理"));
        }
        other => panic!("expected SetMode, got {other:?}"),
    }
}
#[tokio::test]
async fn test_execute_non_plan_mode_with_name_and_instruction() {
    use std::fs;
    let tmp = tempfile::tempdir().unwrap();
    let plans_dir = tmp.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    let plan_file = plans_dir.join("my-plan.md");
    fs::write(&plan_file, "# My Plan\n").unwrap();
    let sm = make_session_manager_with_storage();
    let sid = create_test_session(&sm).await;
    sm.set_workdir(&sid, tmp.path().to_path_buf()).await;
    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    // name only → plan_file_path set, no instruction
    match h.handle("my-plan", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            initial_input,
            ..
        } => {
            assert_eq!(mode, "auto");
            assert!(plan_file_path.is_some());
            assert!(initial_input.is_none());
        }
        other => panic!("expected SetMode, got {other:?}"),
    }
    // name + instruction → both set
    match h.handle("my-plan 请先完成 lint", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            initial_input,
            ..
        } => {
            assert_eq!(mode, "auto");
            assert!(plan_file_path.is_some());
            assert_eq!(initial_input.as_deref(), Some("请先完成 lint"));
        }
        other => panic!("expected SetMode, got {other:?}"),
    }
}

// ── ExecuteHandler error path tests (Step 1.4) ───────────────────────────
// Split into handlers_execute_tests.rs to keep this file under
// the 1000-line limit.
#[path = "handlers_execute_tests.rs"]
mod handlers_execute_tests;
