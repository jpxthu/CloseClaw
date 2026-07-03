//! Unit tests for `SideEffectContext` and `SlashResult::execute()`.
//!
//! Covers: SideEffectContext construction, reply/trigger_compact helpers,
//! and each SlashResult variant's execute() behavior.

use std::sync::Arc;

use crate::handler::{SlashResult, SystemAppendAction};
use crate::side_effect::{ReplyAction, SideEffectContext};
use closeclaw_common::processor::ContentBlock;
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

/// Extract the first `ContentBlock::Text` string from a `Vec<ContentBlock>`.
fn first_text(blocks: &[ContentBlock]) -> String {
    for b in blocks {
        if let ContentBlock::Text(t) = b {
            return t.clone();
        }
    }
    panic!("no ContentBlock::Text found in blocks: {blocks:?}");
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
    ctx.reply(vec![ContentBlock::Text("hello".to_owned())])
        .await;
    let action = rx.recv().await.expect("reply action expected");
    match action {
        ReplyAction::Reply(blocks) => assert_eq!(first_text(&blocks), "hello"),
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
        ReplyAction::Reply(blocks) => assert_eq!(first_text(blocks), "hi there"),
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
        ReplyAction::Reply(blocks) => {
            let text = first_text(blocks);
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
        ReplyAction::Reply(blocks) => {
            let text = first_text(blocks);
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
        ReplyAction::Reply(blocks) => {
            let text = first_text(blocks);
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
        ReplyAction::Reply(blocks) => {
            let text = first_text(blocks);
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
        ReplyAction::Reply(blocks) => {
            let text = first_text(blocks);
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
        ReplyAction::Reply(blocks) => {
            let text = first_text(blocks);
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
        ReplyAction::Reply(blocks) => {
            let text = first_text(blocks);
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
    let _ = format!(
        "{:?}",
        ReplyAction::Reply(vec![ContentBlock::Text("x".to_owned())])
    );
    let _ = format!("{:?}", ReplyAction::TriggerCompact { instruction: None });
    let _ = format!("{:?}", ReplyAction::Nothing);
}

// ---------------------------------------------------------------------------
// Side effects with active sessions
// ---------------------------------------------------------------------------

/// Helper: create a SideEffectContext backed by a real SessionManager
/// with an active conversation session.
async fn make_active_ctx() -> (SideEffectContext, mpsc::Receiver<ReplyAction>, String) {
    let sm = make_sm();
    let msg = closeclaw_gateway::Message {
        id: "se-test-msg".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
    };
    let session_id = sm.find_or_create("feishu", &msg, None).await.unwrap();
    let (reply_tx, reply_rx) = mpsc::channel(16);
    let ctx = SideEffectContext::new(session_id.clone(), "feishu".to_owned(), sm, reply_tx);
    (ctx, reply_rx, session_id)
}

#[tokio::test]
async fn test_execute_set_reasoning_with_session() {
    let (ctx, mut rx, _sid) = make_active_ctx().await;
    let result = SlashResult::SetReasoning {
        level: ReasoningLevel::Low,
    };
    result.execute(&ctx).await;
    drop(ctx);
    let action = rx.recv().await.expect("reply action");
    match action {
        ReplyAction::Reply(blocks) => {
            let text = first_text(&blocks);
            assert!(text.contains("推理深度已设置为"), "got: {text}");
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_set_verbosity_with_session() {
    let (ctx, mut rx, _sid) = make_active_ctx().await;
    let result = SlashResult::SetVerbosity {
        level: closeclaw_common::VerbosityLevel::Normal,
    };
    result.execute(&ctx).await;
    drop(ctx);
    let action = rx.recv().await.expect("reply action");
    match action {
        ReplyAction::Reply(blocks) => {
            let text = first_text(&blocks);
            assert!(text.contains("输出详细度已设置为"), "got: {text}");
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_system_append_add_with_session() {
    let (ctx, mut rx, _sid) = make_active_ctx().await;
    let result = SlashResult::SystemAppend {
        action: SystemAppendAction::Add("new instruction".to_owned()),
    };
    result.execute(&ctx).await;
    drop(ctx);
    let action = rx.recv().await.expect("reply action");
    match action {
        ReplyAction::Reply(blocks) => {
            let text = first_text(&blocks);
            assert!(text.contains("已追加指令"), "got: {text}");
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_system_append_clear_with_session() {
    let (ctx, mut rx, _sid) = make_active_ctx().await;
    let result = SlashResult::SystemAppend {
        action: SystemAppendAction::Clear,
    };
    result.execute(&ctx).await;
    drop(ctx);
    let action = rx.recv().await.expect("reply action");
    match action {
        ReplyAction::Reply(blocks) => {
            let text = first_text(&blocks);
            assert!(text.contains("已清除"), "got: {text}");
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_stop_with_session_not_busy() {
    let (ctx, mut rx, _sid) = make_active_ctx().await;
    let result = SlashResult::Stop;
    result.execute(&ctx).await;
    drop(ctx);
    let action = rx.recv().await.expect("reply action");
    match action {
        ReplyAction::Reply(blocks) => {
            let text = first_text(&blocks);
            assert!(text.contains("已停止当前任务"), "got: {text}");
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// ContentBlock[] verification (Step 1.3)
// ---------------------------------------------------------------------------

/// Verify that ReplyAction::Reply carries Vec<ContentBlock> (not String).
#[test]
fn test_reply_action_reply_is_vec_content_block() {
    let blocks = vec![
        ContentBlock::Text("line1".to_owned()),
        ContentBlock::Text("line2".to_owned()),
    ];
    let action = ReplyAction::Reply(blocks.clone());
    match action {
        ReplyAction::Reply(b) => {
            assert_eq!(b.len(), 2);
            assert_eq!(first_text(&b), "line1");
            assert!(matches!(&b[1], ContentBlock::Text(s) if s == "line2"));
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

/// Execute a Reply slash command and verify the reply carries
/// a single ContentBlock::Text inside the Vec.
#[tokio::test]
async fn test_execute_reply_content_block_text() {
    let actions = execute_and_collect(SlashResult::Reply("test reply".to_owned())).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert!(matches!(&blocks[0], ContentBlock::Text(s) if s == "test reply"));
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

/// Verify that ReplyAction::Reply handles empty content blocks.
#[test]
fn test_reply_action_empty_blocks() {
    let action = ReplyAction::Reply(vec![]);
    match action {
        ReplyAction::Reply(b) => assert!(b.is_empty()),
        other => panic!("expected Reply, got {other:?}"),
    }
}

/// Verify that ReplyAction::Nothing produces no side effects.
#[test]
fn test_reply_action_nothing() {
    let action = ReplyAction::Nothing;
    match action {
        ReplyAction::Nothing => {}
        other => panic!("expected Nothing, got {other:?}"),
    }
}
