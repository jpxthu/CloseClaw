//! Slash command result execution logic.
//!
//! Standalone function that executes a [`SlashResult`], producing
//! [`ReplyAction`]s on the context's reply channel. Moved here from
//! `common::SlashResult::execute()` to keep common free of executable
//! logic.

use closeclaw_common::processor::ContentBlock;
use closeclaw_common::slash_router::{
    ReplyAction, SideEffectContext, SlashResult, SystemAppendAction,
};

/// Execute a [`SlashResult`], producing [`ReplyAction`]s on `ctx.reply_tx`.
///
/// For variants that require side effects (Stop, NewSession, Compact,
/// SystemAppend, SetReasoning, SetVerbosity, Exec), the executor trait on
/// `ctx` is called to perform the actual work.
pub async fn execute_slash_result(result: &SlashResult, ctx: &SideEffectContext) {
    let mut actions = Vec::new();
    match result {
        SlashResult::Reply(text) => {
            actions.push(ReplyAction::Reply(vec![ContentBlock::Text(text.clone())]));
        }
        SlashResult::SetMode(mode) => {
            actions.push(ReplyAction::Reply(vec![ContentBlock::Text(format!(
                "Mode set to: {mode}"
            ))]));
        }
        SlashResult::NewSession => {
            ctx.executor
                .execute_new_session(&ctx.session_id, &ctx.channel)
                .await;
            let _ = ctx
                .reply_tx
                .send(ReplyAction::Reply(vec![ContentBlock::Text(
                    "已创建新 session".into(),
                )]))
                .await;
        }
        SlashResult::Stop => {
            ctx.executor.execute_stop(&ctx.session_id).await;
            let _ = ctx
                .reply_tx
                .send(ReplyAction::Reply(vec![ContentBlock::Text(
                    "已停止当前任务".into(),
                )]))
                .await;
        }
        SlashResult::Compact { instruction } => {
            ctx.executor
                .execute_compact(&ctx.session_id, instruction.clone())
                .await;
        }
        SlashResult::SystemAppend { action } => {
            ctx.executor
                .execute_system_append(&ctx.session_id, action)
                .await;
            match action {
                SystemAppendAction::Add(_) => {
                    let _ = ctx
                        .reply_tx
                        .send(ReplyAction::Reply(vec![ContentBlock::Text(
                            "已追加指令".into(),
                        )]))
                        .await;
                }
                SystemAppendAction::Clear => {
                    let _ = ctx
                        .reply_tx
                        .send(ReplyAction::Reply(vec![ContentBlock::Text(
                            "已清除追加指令".into(),
                        )]))
                        .await;
                }
            }
        }
        SlashResult::Exec { command } => {
            let agent_id = ctx
                .session_manager
                .get_chat_id(&ctx.session_id)
                .await
                .unwrap_or_default();
            let blocks = ctx
                .executor
                .execute_exec(&ctx.session_id, &agent_id, command)
                .await;
            actions.push(ReplyAction::Reply(blocks));
        }
        SlashResult::SetReasoning { level } => {
            ctx.executor
                .execute_set_reasoning(&ctx.session_id, *level)
                .await;
            let _ = ctx
                .reply_tx
                .send(ReplyAction::Reply(vec![ContentBlock::Text(format!(
                    "推理深度已设置为 {level}"
                ))]))
                .await;
        }
        SlashResult::SetVerbosity { level } => {
            ctx.executor
                .execute_set_verbosity(&ctx.session_id, *level)
                .await;
            let _ = ctx
                .reply_tx
                .send(ReplyAction::Reply(vec![ContentBlock::Text(format!(
                    "输出详细度已设置为 {level}"
                ))]))
                .await;
        }
        SlashResult::Unknown(cmd) => {
            actions.push(ReplyAction::Reply(vec![ContentBlock::Text(format!(
                "Unknown command: /{cmd}"
            ))]));
        }
    }
    for action in actions {
        let _ = ctx.reply_tx.send(action).await;
    }
}
