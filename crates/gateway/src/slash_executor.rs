//! Extension trait for executing [`SlashResult`] side effects.
//!
//! Defines [`SlashResultExecutor`], an extension trait on
//! [`SlashResult`] that performs the actual side-effect dispatch
//! through a [`SideEffectContext`]. This keeps executable logic
//! out of the `common` crate (which only defines data structures
//! and trait signatures) and places it in the `gateway` crate,
//! which owns the concrete session and permission implementations.

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

use closeclaw_common::processor::ContentBlock;
use closeclaw_common::slash_router::{SlashResult, SystemAppendAction};
use closeclaw_common::SessionLookup;
use closeclaw_common::{ReasoningLevel, VerbosityLevel};

// ── Migrated types (from common) ──────────────────────────────────────

/// Action produced by execute_slash_result for the Gateway to dispatch.
#[derive(Debug)]
pub enum ReplyAction {
    /// Send a content-block reply to the user (routed through outbound
    /// Processor Chain: Verbosity filtering → DslParser → outbound logging
    /// → IM Adapter rendering).
    Reply(Vec<ContentBlock>),
    /// Trigger manual compaction.
    TriggerCompact { instruction: Option<String> },
    /// No action needed.
    Nothing,
}

/// Executor trait for slash command side effects.
///
/// Implemented by the Gateway, which has access to the full
/// `SessionManager` and `SessionMessageHandler`. This trait breaks
/// the circular dependency: common defines the interface, gateway
/// provides the implementation.
#[async_trait]
pub trait SlashEffectExecutor: Send + Sync {
    /// Stop the current LLM turn for the session.
    async fn execute_stop(&self, session_id: &str);

    /// Create a new session for the given channel.
    async fn execute_new_session(&self, session_id: &str, channel: &str);

    /// Trigger context compaction with an optional custom instruction.
    async fn execute_compact(&self, session_id: &str, instruction: Option<String>);

    /// Apply a system prompt append/clear action.
    async fn execute_system_append(&self, session_id: &str, action: &SystemAppendAction);

    /// Set the reasoning level for the session.
    async fn execute_set_reasoning(&self, session_id: &str, level: ReasoningLevel);

    /// Set the verbosity level for the session.
    async fn execute_set_verbosity(&self, session_id: &str, level: VerbosityLevel);

    /// Execute a shell command for the given agent.
    ///
    /// The implementation evaluates command-level permissions via the
    /// permission engine, then runs the command and returns output as
    /// `ContentBlock::Text`. Returns a rejection message on permission denial.
    async fn execute_exec(
        &self,
        session_id: &str,
        agent_id: &str,
        command: &str,
    ) -> Vec<ContentBlock>;
}

/// Context for slash command side-effect dispatch.
///
/// Carries session/channel identity, a reply channel, and an executor
/// for the Gateway to dispatch side effects.
pub struct SideEffectContext {
    /// Session ID where the slash command was invoked.
    pub session_id: String,
    /// Channel identifier (e.g. "feishu", "telegram").
    pub channel: String,
    /// Session manager for state queries.
    pub session_manager: Arc<dyn SessionLookup>,
    /// Sender for [`ReplyAction`]s.
    pub reply_tx: mpsc::Sender<ReplyAction>,
    /// Executor for slash command side effects.
    pub executor: Arc<dyn SlashEffectExecutor>,
}

/// Extension trait for executing [`SlashResult`] side effects.
///
/// Implemented for [`SlashResult`] in the gateway crate. The gateway
/// calls `result.execute(&ctx).await` after constructing a
/// [`SideEffectContext`] with the appropriate executor and reply
/// channel.
#[async_trait]
pub trait SlashResultExecutor {
    /// Execute this slash result, performing side effects through `ctx`.
    ///
    /// Each [`SlashResult`] variant dispatches to the corresponding
    /// [`SideEffectContext`] method and sends reply actions on
    /// `ctx.reply_tx`.
    async fn execute(self, ctx: &SideEffectContext);
}

#[async_trait]
impl SlashResultExecutor for SlashResult {
    async fn execute(self, ctx: &SideEffectContext) {
        let mut actions = Vec::new();
        match self {
            SlashResult::Reply(text) => {
                actions.push(ReplyAction::Reply(vec![ContentBlock::Text(text)]));
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
                    .execute_compact(&ctx.session_id, instruction)
                    .await;
                let _ = ctx
                    .reply_tx
                    .send(ReplyAction::Reply(vec![ContentBlock::Text(
                        "对话历史已压缩".into(),
                    )]))
                    .await;
            }
            SlashResult::SystemAppend { action } => {
                ctx.executor
                    .execute_system_append(&ctx.session_id, &action)
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
                    .execute_exec(&ctx.session_id, &agent_id, &command)
                    .await;
                actions.push(ReplyAction::Reply(blocks));
            }
            SlashResult::SetReasoning { level } => {
                ctx.executor
                    .execute_set_reasoning(&ctx.session_id, level)
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
                    .execute_set_verbosity(&ctx.session_id, level)
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
}
