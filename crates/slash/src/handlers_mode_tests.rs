//! Unit tests for PlanModeHandler and ModeHandler.

use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use crate::handlers_mode::{ModeHandler, PlanModeHandler};
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
    let h = PlanModeHandler;
    assert_eq!(h.commands(), &["plan"]);
    assert_eq!(h.description(), "进入 Plan Mode");
}

#[test]
fn test_plan_mode_handler_not_immediate() {
    let h = PlanModeHandler;
    assert!(!h.immediate("plan"));
}

#[tokio::test]
async fn test_plan_mode_handler_with_args_returns_set_mode() {
    let h = PlanModeHandler;
    let ctx = dummy_ctx();
    match h.handle("实现一个新功能", &ctx).await {
        SlashResult::SetMode(mode) => assert_eq!(mode, "plan"),
        other => panic!("expected SetMode(\"plan\"), got {other:?}"),
    }
}

#[tokio::test]
async fn test_plan_mode_handler_with_whitespace_args_returns_set_mode() {
    let h = PlanModeHandler;
    let ctx = dummy_ctx();
    match h.handle("  优化性能  ", &ctx).await {
        SlashResult::SetMode(mode) => assert_eq!(mode, "plan"),
        other => panic!("expected SetMode(\"plan\"), got {other:?}"),
    }
}

#[tokio::test]
async fn test_plan_mode_handler_no_args_returns_usage() {
    let h = PlanModeHandler;
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
    let h = PlanModeHandler;
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
        SlashResult::SetMode(mode) => assert_eq!(mode, "plan"),
        other => panic!("expected SetMode(\"plan\"), got {other:?}"),
    }
}

#[tokio::test]
async fn test_mode_handler_set_auto() {
    let sm = make_session_manager();
    let h = ModeHandler::new(sm);
    let ctx = dummy_ctx();
    match h.handle("auto", &ctx).await {
        SlashResult::SetMode(mode) => assert_eq!(mode, "auto"),
        other => panic!("expected SetMode(\"auto\"), got {other:?}"),
    }
}

#[tokio::test]
async fn test_mode_handler_set_normal() {
    let sm = make_session_manager();
    let h = ModeHandler::new(sm);
    let ctx = dummy_ctx();
    match h.handle("normal", &ctx).await {
        SlashResult::SetMode(mode) => assert_eq!(mode, "normal"),
        other => panic!("expected SetMode(\"normal\"), got {other:?}"),
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
        SlashResult::SetMode(mode) => assert_eq!(mode, "plan"),
        other => panic!("expected SetMode(\"plan\") with whitespace, got {other:?}"),
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
