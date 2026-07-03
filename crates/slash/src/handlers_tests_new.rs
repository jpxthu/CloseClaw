//! Tests for NewSessionHandler, StopHandler, StatusHandler, and /help inclusion.

use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use crate::registry::HandlerRegistry;
use crate::{HelpHandler, NewSessionHandler, StatusHandler, StopHandler};
use closeclaw_common::slash_router::SlashResult;
use closeclaw_gateway::session_manager::SessionManager;

// ── Shared helpers ─────────────────────────────────────────────────────────

pub(crate) fn dummy_ctx() -> SlashContext {
    SlashContext {
        command: String::new(),
        sender_id: "test_sender".to_owned(),
        session_id: "test_session".to_owned(),
        channel: "test_channel".to_owned(),
    }
}

fn make_workdir_session_manager() -> std::sync::Arc<SessionManager> {
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

async fn create_test_session(sm: &SessionManager) -> String {
    use closeclaw_gateway::Message;

    let msg = Message {
        id: "workdir-test-msg-1".to_string(),
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

// ── NewSessionHandler tests ────────────────────────────────────────────────

#[test]
fn test_new_session_handler_commands() {
    let h = NewSessionHandler;
    assert_eq!(h.commands(), &["new"]);
}

#[test]
fn test_new_session_handler_immediate() {
    assert!(!NewSessionHandler.immediate("new"));
}

#[tokio::test]
async fn test_new_session_handler_handle() {
    let result = NewSessionHandler.handle("", &dummy_ctx()).await;
    assert!(matches!(result, SlashResult::NewSession));
}

// ── StopHandler tests ─────────────────────────────────────────────────────

#[test]
fn test_stop_handler_commands() {
    let h = StopHandler;
    assert_eq!(h.commands(), &["stop"]);
}

#[test]
fn test_stop_handler_immediate() {
    assert!(StopHandler.immediate("stop"));
}

#[tokio::test]
async fn test_stop_handler_handle() {
    let result = StopHandler.handle("", &dummy_ctx()).await;
    assert!(matches!(result, SlashResult::Stop));
}

// ── StatusHandler tests ────────────────────────────────────────────────────

#[test]
fn test_status_handler_commands() {
    let h = StatusHandler::new(make_workdir_session_manager());
    assert_eq!(h.commands(), &["status"]);
}

#[test]
fn test_status_handler_immediate() {
    assert!(StatusHandler::new(make_workdir_session_manager()).immediate("status"));
}

#[tokio::test]
async fn test_status_handler_no_session() {
    let h = StatusHandler::new(make_workdir_session_manager());
    let ctx = SlashContext {
        command: "status".to_owned(),
        sender_id: "test_sender".to_owned(),
        session_id: "nonexistent_session".to_owned(),
        channel: "test_channel".to_owned(),
    };
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => assert_eq!(t, "当前会话未激活", "got: {t}"),
        _ => panic!("expected Reply with no-session message"),
    }
}

#[tokio::test]
async fn test_status_handler_with_session() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = StatusHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => {
            assert!(t.contains("LLM 状态"), "missing LLM status, got: {t}");
            assert!(t.contains("模型"), "missing model, got: {t}");
            assert!(t.contains("推理深度"), "missing reasoning, got: {t}");
            assert!(t.contains("上下文用量"), "missing tokens, got: {t}");
            assert!(t.contains("活跃子 agent"), "missing children, got: {t}");
            assert!(t.contains("工作目录"), "missing workdir, got: {t}");
            assert!(t.contains("追加指令"), "missing appends, got: {t}");
        }
        _ => panic!("expected Reply with status fields"),
    }
}

// ── /help includes new, stop, status ──────────────────────────────────────

#[tokio::test]
async fn test_help_includes_new_stop_status() {
    let registry = Arc::new(HandlerRegistry::new());
    registry.register(Arc::new(NewSessionHandler));
    registry.register(Arc::new(StopHandler));
    registry.register(Arc::new(StatusHandler::new(make_workdir_session_manager())));
    let help = HelpHandler::new(Arc::clone(&registry));
    let ctx = dummy_ctx();
    match help.handle("", &ctx).await {
        SlashResult::Reply(t) => {
            assert!(t.contains("/new"), "missing /new, got: {t}");
            assert!(t.contains("/stop"), "missing /stop, got: {t}");
            assert!(t.contains("/status"), "missing /status, got: {t}");
        }
        _ => panic!("expected Reply"),
    }
}
