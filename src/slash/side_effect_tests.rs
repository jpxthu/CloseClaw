//! Unit tests for `SideEffectContext` and `SlashResult::execute()`.
//!
//! Covers: SideEffectContext construction, reply/trigger_compact helpers,
//! and each SlashResult variant's execute() behavior.

use std::sync::Arc;

use crate::slash::handler::{SlashResult, SystemAppendAction};
use crate::slash::side_effect::{ReplyAction, SideEffectContext};
use closeclaw_gateway::session_manager::SessionManager;
use closeclaw_gateway::{DmScope, GatewayConfig};
use closeclaw_session::bootstrap::loader::BootstrapMode;
use closeclaw_session::persistence::ReasoningLevel;
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_sm() -> Arc<SessionManager> {
    let config = GatewayConfig {
        name: "test".to_owned(),
        dm_scope: DmScope::default(),
        ..Default::default()
    };
    Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    ))
}

fn make_ctx() -> (SideEffectContext, mpsc::Receiver<ReplyAction>) {
    let (reply_tx, reply_rx) = mpsc::channel(16);
    let ctx = SideEffectContext::new("sess1".to_owned(), "feishu".to_owned(), make_sm(), reply_tx);
    (ctx, reply_rx)
}

/// Helper: execute a SlashResult and collect all reply actions.
async fn execute_and_collect(result: SlashResult) -> Vec<ReplyAction> {
    let (ctx, mut rx) = make_ctx();
    result.execute(&ctx).await;
    drop(ctx);
    let mut actions = Vec::new();
    while let Some(a) = rx.recv().await {
        actions.push(a);
    }
    actions
}

// ---------------------------------------------------------------------------
// SideEffectContext tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_side_effect_context_construction() {
    let (ctx, _rx) = make_ctx();
    assert_eq!(ctx.session_id, "sess1");
    assert_eq!(ctx.channel, "feishu");
}

#[tokio::test]
async fn test_side_effect_context_reply() {
    let (ctx, mut rx) = make_ctx();
    ctx.reply("hello".to_owned()).await;
    let action = rx.recv().await.expect("reply action expected");
    match action {
        ReplyAction::Reply(text) => assert_eq!(text, "hello"),
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_side_effect_context_trigger_compact() {
    let (ctx, mut rx) = make_ctx();
    ctx.trigger_compact(Some("keep summary".to_owned())).await;
    let action = rx.recv().await.expect("compact action expected");
    match action {
        ReplyAction::TriggerCompact { instruction } => {
            assert_eq!(instruction, Some("keep summary".to_owned()));
        }
        other => panic!("expected TriggerCompact, got {other:?}"),
    }
}

#[tokio::test]
async fn test_side_effect_context_trigger_compact_no_instruction() {
    let (ctx, mut rx) = make_ctx();
    ctx.trigger_compact(None).await;
    let action = rx.recv().await.expect("compact action expected");
    match action {
        ReplyAction::TriggerCompact { instruction } => {
            assert_eq!(instruction, None);
        }
        other => panic!("expected TriggerCompact with None, got {other:?}"),
    }
}

#[tokio::test]
async fn test_side_effect_context_get_conversation_session_none() {
    let (ctx, _rx) = make_ctx();
    // No session created in the manager — should return None.
    let cs = ctx.get_conversation_session().await;
    assert!(cs.is_none());
}

// ---------------------------------------------------------------------------
// SlashResult::execute() — Reply variant
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_execute_reply() {
    let actions = execute_and_collect(SlashResult::Reply("hi there".to_owned())).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(text) => assert_eq!(text, "hi there"),
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// SlashResult::execute() — Compact variant
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_execute_compact_with_instruction() {
    let result = SlashResult::Compact {
        instruction: Some("retain API list".to_owned()),
    };
    let actions = execute_and_collect(result).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::TriggerCompact { instruction } => {
            assert_eq!(instruction.as_deref(), Some("retain API list"));
        }
        other => panic!("expected TriggerCompact, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_compact_without_instruction() {
    let result = SlashResult::Compact { instruction: None };
    let actions = execute_and_collect(result).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::TriggerCompact { instruction } => assert!(instruction.is_none()),
        other => panic!("expected TriggerCompact, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// SlashResult::execute() — Exec variant
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_execute_exec() {
    let result = SlashResult::Exec {
        command: "rm -rf /tmp/test".to_owned(),
    };
    let actions = execute_and_collect(result).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(text) => {
            assert!(text.contains("rm -rf /tmp/test"), "got: {text}");
            assert!(text.contains("已提交审批"), "got: {text}");
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// SlashResult::execute() — SetReasoning variant
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_execute_set_reasoning_no_session() {
    let result = SlashResult::SetReasoning {
        level: ReasoningLevel::High,
    };
    let actions = execute_and_collect(result).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(text) => {
            assert!(
                text.contains("当前会话未激活"),
                "expected no-session message, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// SlashResult::execute() — SetVerbosity variant
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_execute_set_verbosity_no_session() {
    let result = SlashResult::SetVerbosity {
        level: closeclaw_common::VerbosityLevel::Off,
    };
    let actions = execute_and_collect(result).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(text) => {
            assert!(
                text.contains("当前会话未激活"),
                "expected no-session message, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// SlashResult::execute() — Unknown variant
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_execute_unknown_no_side_effects() {
    let actions = execute_and_collect(SlashResult::Unknown("xyz".to_owned())).await;
    assert!(
        actions.is_empty(),
        "Unknown variant should produce no reply actions, got: {actions:?}"
    );
}

// ---------------------------------------------------------------------------
// SlashResult::execute() — SetMode variant
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_execute_set_mode_no_side_effects() {
    // SetMode is a future variant — currently logs a warning but produces
    // no reply actions.
    let actions = execute_and_collect(SlashResult::SetMode("dark".to_owned())).await;
    assert!(
        actions.is_empty(),
        "SetMode variant should produce no reply actions, got: {actions:?}"
    );
}

// ---------------------------------------------------------------------------
// SlashResult::execute() — SystemAppend variant
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_execute_system_append_add_no_session() {
    let result = SlashResult::SystemAppend {
        action: SystemAppendAction::Add("new instruction".to_owned()),
    };
    let actions = execute_and_collect(result).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(text) => {
            assert!(
                text.contains("当前会话未激活"),
                "expected no-session message, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_system_append_clear_no_session() {
    let result = SlashResult::SystemAppend {
        action: SystemAppendAction::Clear,
    };
    let actions = execute_and_collect(result).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(text) => {
            assert!(
                text.contains("当前会话未激活"),
                "expected no-session message, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// SlashResult::execute() — NewSession variant
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_execute_new_session() {
    let result = SlashResult::NewSession;
    let actions = execute_and_collect(result).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(text) => {
            assert!(
                text.contains("已创建新 session"),
                "expected new-session message, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// SlashResult::execute() — Stop variant
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_execute_stop_no_session() {
    let result = SlashResult::Stop;
    let actions = execute_and_collect(result).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(text) => {
            assert!(
                text.contains("当前会话未激活"),
                "expected no-session message, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// ReplyAction enum coverage
// ---------------------------------------------------------------------------

#[test]
fn test_reply_action_debug() {
    // Ensure Debug is implemented and doesn't panic.
    let _ = format!("{:?}", ReplyAction::Reply("x".to_owned()));
    let _ = format!("{:?}", ReplyAction::TriggerCompact { instruction: None });
    let _ = format!("{:?}", ReplyAction::Nothing);
}
