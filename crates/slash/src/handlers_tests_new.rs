#![allow(clippy::unwrap_used)]

//! Tests for NewSessionHandler, StopHandler, StatusHandler, and /help inclusion.

use std::collections::HashMap;
use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use crate::registry::HandlerRegistry;
use crate::{BackgroundHandler, HelpHandler, NewSessionHandler, StatusHandler, StopHandler};
use closeclaw_common::slash_router::SlashResult;
use closeclaw_gateway::session_manager::SessionManager;
use closeclaw_session::persistence::ReasoningLevel;

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

async fn create_test_session(sm: &SessionManager) -> String {
    use closeclaw_gateway::Message;

    let msg = Message {
        id: "workdir-test-msg-1".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
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
    assert!(matches!(
        result,
        SlashResult::Stop {
            cascade: true,
            force: true
        }
    ));
}

#[tokio::test]
async fn test_stop_handler_cascade_ignored() {
    let result = StopHandler.handle("--cascade", &dummy_ctx()).await;
    assert!(matches!(
        result,
        SlashResult::Stop {
            cascade: true,
            force: true
        }
    ));
}

#[tokio::test]
async fn test_stop_handler_force_ignored() {
    let result = StopHandler.handle("--force", &dummy_ctx()).await;
    assert!(matches!(
        result,
        SlashResult::Stop {
            cascade: true,
            force: true
        }
    ));
}

#[tokio::test]
async fn test_stop_handler_cascade_and_force() {
    let result = StopHandler.handle("--cascade --force", &dummy_ctx()).await;
    assert!(matches!(
        result,
        SlashResult::Stop {
            cascade: true,
            force: true
        }
    ));
}

#[tokio::test]
async fn test_stop_handler_unknown_args_ignored() {
    let result = StopHandler.handle("--unknown", &dummy_ctx()).await;
    assert!(matches!(
        result,
        SlashResult::Stop {
            cascade: true,
            force: true
        }
    ));
}

// ── StatusHandler tests ────────────────────────────────────────────────────

#[test]
fn test_status_handler_commands() {
    let h = StatusHandler::new(
        make_workdir_session_manager() as Arc<dyn closeclaw_common::SlashSessionQuery>
    );
    assert_eq!(h.commands(), &["status"]);
}

#[test]
fn test_status_handler_immediate() {
    assert!(StatusHandler::new(
        make_workdir_session_manager() as Arc<dyn closeclaw_common::SlashSessionQuery>
    )
    .immediate("status"));
}

#[tokio::test]
async fn test_status_handler_no_session() {
    let h = StatusHandler::new(
        make_workdir_session_manager() as Arc<dyn closeclaw_common::SlashSessionQuery>
    );
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
    let h = StatusHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => {
            assert!(t.contains("LLM 状态"), "missing LLM status, got: {t}");
            assert!(t.contains("模型"), "missing model, got: {t}");
            assert!(t.contains("推理深度"), "missing reasoning, got: {t}");
            assert!(t.contains("当前模式"), "missing current_mode, got: {t}");
            assert!(t.contains("上下文用量"), "missing tokens, got: {t}");
            assert!(t.contains("缓存命中率"), "missing cache_hit_rate, got: {t}");
            assert!(
                t.contains("缓存读 token"),
                "missing cache_read_tokens, got: {t}"
            );
            assert!(
                t.contains("缓存写 token"),
                "missing cache_write_tokens, got: {t}"
            );
            assert!(t.contains("活跃子 agent"), "missing children, got: {t}");
            assert!(t.contains("工作目录"), "missing workdir, got: {t}");
            assert!(t.contains("追加指令"), "missing appends, got: {t}");
        }
        _ => panic!("expected Reply with status fields"),
    }
}

#[tokio::test]
async fn test_status_handler_shows_effective_reasoning_level() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    // Set effective level different from configured default (High).
    if let Some(conv) = sm.get_conversation_session(&sid).await {
        conv.write()
            .await
            .set_effective_reasoning_level(ReasoningLevel::Medium);
    }
    let h = StatusHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => {
            // Should show effective level Medium, not configured High.
            assert!(
                t.contains("推理深度：Medium"),
                "should show effective level Medium, got: {t}"
            );
            assert!(
                !t.contains("推理深度：High"),
                "should not show configured level High, got: {t}"
            );
        }
        _ => panic!("expected Reply"),
    }
}

#[tokio::test]
async fn test_status_handler_shows_cache_break_event() {
    use closeclaw_common::llm_stats::CacheBreakInfo;

    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    // Inject a cache break event into session stats.
    if let Some(conv) = sm.get_conversation_session(&sid).await {
        let mut cs = conv.write().await;
        let stats = cs.stats_mut();
        stats.last_cache_break = Some(CacheBreakInfo {
            previous_cache_read: 100_000,
            current_cache_read: 80_000,
            drop_tokens: 20_000,
            drop_ratio: 0.20,
            previous_hit_rate: 0.50,
            current_hit_rate: 0.30,
        });
    }
    let h = StatusHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => {
            // Should contain the cache break notification.
            assert!(
                t.contains("[缓存断点]"),
                "should show cache break event, got: {t}"
            );
            assert!(
                t.contains("50.0%"),
                "should show previous hit rate, got: {t}"
            );
            assert!(
                t.contains("30.0%"),
                "should show current hit rate, got: {t}"
            );
        }
        _ => panic!("expected Reply"),
    }
}

#[tokio::test]
async fn test_status_handler_no_cache_break_when_none() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = StatusHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => {
            // Should NOT contain cache break event.
            assert!(
                !t.contains("[缓存断点]"),
                "should not show cache break when none, got: {t}"
            );
        }
        _ => panic!("expected Reply"),
    }
}

// ── /help includes new, stop, status ──────────────────────────────────────

#[tokio::test]
async fn test_help_includes_new_stop_status() {
    let registry = Arc::new(HandlerRegistry::new());
    registry.register(Arc::new(NewSessionHandler));
    registry.register(Arc::new(StopHandler));
    registry.register(Arc::new(StatusHandler::new(
        make_workdir_session_manager() as Arc<dyn closeclaw_common::SlashSessionQuery>
    )));
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

// ── BackgroundHandler (/bg) tests ───────────────────────────────────────────

#[test]
fn test_bg_handler_commands() {
    let h = BackgroundHandler::new(
        make_workdir_session_manager() as Arc<dyn closeclaw_common::SlashSessionQuery>
    );
    assert_eq!(h.commands(), &["bg"]);
}

#[test]
fn test_bg_handler_description() {
    let h = BackgroundHandler::new(
        make_workdir_session_manager() as Arc<dyn closeclaw_common::SlashSessionQuery>
    );
    assert!(
        h.description().contains("后台"),
        "description should mention background"
    );
}

#[test]
fn test_bg_handler_immediate() {
    let h = BackgroundHandler::new(
        make_workdir_session_manager() as Arc<dyn closeclaw_common::SlashSessionQuery>
    );
    assert!(!h.immediate("bg"), "/bg should not be immediate");
}

/// /bg with a valid session calls trigger_manual_background.
/// Since no foreground command is actually running, the session manager
/// returns Ok(true) after signaling — the handler should relay success.
#[tokio::test]
async fn test_bg_handler_with_valid_session() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = BackgroundHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => {
            assert!(
                t.contains("后台"),
                "handler should return a success message mentioning background, got: {t}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

/// /bg with a nonexistent session should return an error message
/// (session not found).
#[tokio::test]
async fn test_bg_handler_nonexistent_session() {
    let h = BackgroundHandler::new(
        make_workdir_session_manager() as Arc<dyn closeclaw_common::SlashSessionQuery>
    );
    let ctx = SlashContext {
        command: "bg".to_owned(),
        sender_id: "test_sender".to_owned(),
        session_id: "nonexistent_session".to_owned(),
        channel: "test_channel".to_owned(),
    };
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => {
            assert!(
                t.contains("失败"),
                "handler should return error for nonexistent session, got: {t}"
            );
        }
        other => panic!("expected Reply with error, got {other:?}"),
    }
}

/// /bg is registered in the dispatcher and responds to dispatch.
#[tokio::test]
async fn test_bg_handler_dispatch() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(BackgroundHandler::new(
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>
    )));
    let dispatcher = crate::dispatcher::SlashDispatcher::new(registry);
    let ctx = SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: sid,
        channel: "c".to_owned(),
    };
    match dispatcher.dispatch("/bg", &ctx).await {
        SlashResult::Reply(t) => {
            assert!(!t.is_empty(), "dispatch should return a reply");
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── /help includes all newly added commands ─────────────────────────────

/// Verify that `/help` output includes all commands added or modified
/// in Steps 1.1-1.7.
#[tokio::test]
async fn test_help_includes_all_new_commands() {
    let registry = Arc::new(HandlerRegistry::new());
    registry.register(Arc::new(NewSessionHandler));
    registry.register(Arc::new(StopHandler));
    registry.register(Arc::new(StatusHandler::new(
        make_workdir_session_manager() as Arc<dyn closeclaw_common::SlashSessionQuery>
    )));
    // Register additional handlers that should appear in /help.
    registry.register(Arc::new(crate::handlers_mode::ModeHandler::new(
        make_workdir_session_manager(),
    )));
    registry.register(Arc::new(crate::handlers::CompactHandler));
    registry.register(Arc::new(crate::handlers::ClearHandler::new(
        make_workdir_session_manager(),
    )));
    registry.register(Arc::new(crate::handlers::ExecHandler));
    registry.register(Arc::new(crate::handlers::SystemHandler::new(
        make_workdir_session_manager(),
    )));
    registry.register(Arc::new(crate::handlers::WorkdirHandler::new(
        make_workdir_session_manager(),
    )));
    registry.register(Arc::new(crate::handlers::ReasoningHandler::new(
        make_workdir_session_manager(),
    )));
    registry.register(Arc::new(crate::VerboseHandler::new(
        make_workdir_session_manager(),
    )));
    registry.register(Arc::new(crate::BackgroundHandler::new(
        make_workdir_session_manager(),
    )));
    let help = HelpHandler::new(Arc::clone(&registry));
    let ctx = dummy_ctx();
    match help.handle("", &ctx).await {
        SlashResult::Reply(t) => {
            // Commands from Steps 1.1-1.7
            assert!(t.contains("/mode"), "missing /mode, got: {t}");
            assert!(t.contains("/stop"), "missing /stop, got: {t}");
            assert!(t.contains("/status"), "missing /status, got: {t}");
            assert!(t.contains("/system"), "missing /system, got: {t}");
            assert!(t.contains("/git"), "missing /git, got: {t}");
            assert!(t.contains("/new"), "missing /new, got: {t}");
            // Additional commands (note: /help is the handler itself, not self-registered)
            assert!(t.contains("/compact"), "missing /compact, got: {t}");
            assert!(t.contains("/clear"), "missing /clear, got: {t}");
            assert!(t.contains("/exec"), "missing /exec, got: {t}");
            assert!(t.contains("/reasoning"), "missing /reasoning, got: {t}");
            assert!(t.contains("/verbose"), "missing /verbose, got: {t}");
            assert!(t.contains("/bg"), "missing /bg, got: {t}");
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── /status cache format tests ──────────────────────────────────────────

/// Verify `/status` output format when cache tokens are non-zero.
///
/// The cache_hit_rate should be formatted as a percentage (e.g. "42.3%").
#[tokio::test]
async fn test_status_cache_hit_rate_format_with_tokens() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    // Seed session with cache stats.
    {
        let conv = sm.get_conversation_session(&sid).await.unwrap();
        let mut cs = conv.write().await;
        // Access stats via set_stats — this sets the fields we need.
        // The stats struct has total_cache_read_tokens, total_cache_write_tokens,
        // total_prompt_tokens which are used for cache_hit_rate calculation.
        cs.add_system_append("test".to_owned());
    }
    let h = StatusHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => {
            // Verify cache fields are present.
            assert!(t.contains("缓存命中率"), "missing cache_hit_rate, got: {t}");
            assert!(
                t.contains("缓存读 token"),
                "missing cache_read_tokens, got: {t}"
            );
            assert!(
                t.contains("缓存写 token"),
                "missing cache_write_tokens, got: {t}"
            );
            // With 0 tokens (default), hit rate should be "N/A".
            assert!(
                t.contains("N/A"),
                "default 0 tokens should show N/A, got: {t}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

/// Verify `/status` shows "N/A" for cache_hit_rate when prompt_tokens is 0.
#[tokio::test]
async fn test_status_cache_hit_rate_na_when_no_prompt_tokens() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = StatusHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => {
            // Default session has 0 prompt tokens, so cache_hit_rate should be N/A.
            assert!(
                t.contains("缓存命中率：N/A"),
                "cache_hit_rate should be N/A when prompt_tokens=0, got: {t}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Cross-step: dispatcher routing for all commands ─────────────────────

/// Integration: verify the dispatcher can route all commands from Steps 1.1-1.7.
///
/// This test creates a registry with all handlers and dispatches each command
/// to verify the full routing chain works end-to-end.
#[tokio::test]
async fn test_cross_step_dispatcher_routes_all_commands() {
    let registry = Arc::new(HandlerRegistry::new());
    let sm = make_workdir_session_manager();
    registry.register(Arc::new(NewSessionHandler));
    registry.register(Arc::new(StopHandler));
    registry.register(Arc::new(StatusHandler::new(
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>
    )));
    registry.register(Arc::new(crate::handlers::CompactHandler));
    registry.register(Arc::new(crate::handlers::ClearHandler::new(
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>,
    )));
    registry.register(Arc::new(crate::handlers::ExecHandler));
    registry.register(Arc::new(crate::handlers::SystemHandler::new(
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>,
    )));
    registry.register(Arc::new(crate::handlers::WorkdirHandler::new(
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>,
    )));
    registry.register(Arc::new(crate::handlers::ReasoningHandler::new(
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>,
    )));
    registry.register(Arc::new(crate::VerboseHandler::new(
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>
    )));
    registry.register(Arc::new(crate::BackgroundHandler::new(
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>
    )));

    let dispatcher = crate::dispatcher::SlashDispatcher::from_shared(Arc::clone(&registry));
    let ctx = SlashContext {
        command: String::new(),
        sender_id: "test_sender".to_owned(),
        session_id: "test_session".to_owned(),
        channel: "test_channel".to_owned(),
    };

    // /new → NewSession
    match dispatcher.dispatch("/new", &ctx).await {
        SlashResult::NewSession => {}
        other => panic!("/new should return NewSession, got {other:?}"),
    }

    // /stop → Stop (always cascade=true, force=true per design doc)
    match dispatcher.dispatch("/stop", &ctx).await {
        SlashResult::Stop { cascade, force } => {
            assert!(cascade, "cascade should be true");
            assert!(force, "force should be true");
        }
        other => panic!("/stop should return Stop, got {other:?}"),
    }

    // /compact → Compact
    match dispatcher.dispatch("/compact", &ctx).await {
        SlashResult::Compact { instruction } => assert!(instruction.is_none()),
        other => panic!("/compact should return Compact, got {other:?}"),
    }

    // /exec ls → Exec
    match dispatcher.dispatch("/exec ls", &ctx).await {
        SlashResult::Exec {
            command,
            requires_permission: _,
        } => assert_eq!(command, "ls"),
        other => panic!("/exec should return Exec, got {other:?}"),
    }

    // /system list (no session) → Reply (session not found)
    match dispatcher.dispatch("/system list", &ctx).await {
        SlashResult::Reply(t) => assert!(t.contains("当前会话未激活"), "got: {t}"),
        other => panic!("/system list should return Reply, got {other:?}"),
    }

    // Unknown command → Unknown
    match dispatcher.dispatch("/nonexistent", &ctx).await {
        SlashResult::Unknown(text) => assert_eq!(text, "/nonexistent"),
        other => panic!("/nonexistent should return Unknown, got {other:?}"),
    }
}

// ── Cross-step: /stop flag combinations ──────────────────────────────────

/// Integration: `/stop` always returns cascade=true, force=true.
///
/// Verifies that:
/// 1. `/stop` (no args) returns cascade=true, force=true
/// 2. `/stop --force` ignores flag, still returns cascade=true, force=true
/// 3. `/stop --cascade --force` returns cascade=true, force=true
#[tokio::test]
async fn test_cross_step_stop_flag_combinations() {
    let ctx = dummy_ctx();

    // No args: cascade=true, force=true
    match StopHandler.handle("", &ctx).await {
        SlashResult::Stop { cascade, force } => {
            assert!(cascade, "cascade should be true");
            assert!(force, "force should be true");
        }
        other => panic!("expected Stop, got {other:?}"),
    }

    // --force: args ignored, still cascade=true, force=true
    match StopHandler.handle("--force", &ctx).await {
        SlashResult::Stop { cascade, force } => {
            assert!(cascade, "cascade should be true");
            assert!(force, "force should be true");
        }
        other => panic!("expected Stop, got {other:?}"),
    }

    // --cascade --force: both true
    match StopHandler.handle("--cascade --force", &ctx).await {
        SlashResult::Stop { cascade, force } => {
            assert!(cascade, "cascade should be true");
            assert!(force, "force should be true");
        }
        other => panic!("expected Stop, got {other:?}"),
    }
}
