//! Execution context for slash command side effects.
//!
//! [`SideEffectContext`] encapsulates the session reference and reply channel
//! so that each [`SlashResult`](super::SlashResult) variant can complete its
//! own side effects without the Gateway enumerating every variant.

use std::sync::Arc;

use closeclaw_common::processor::ContentBlock;
use closeclaw_gateway::session_manager::SessionManager;
use closeclaw_llm::session::ChatSession;

/// Action produced by [`SlashResult::execute`](super::SlashResult::execute)
/// for the Gateway to dispatch.
#[derive(Debug)]
pub enum ReplyAction {
    /// Send a content-block reply to the user (routed through outbound
    /// Processor Chain: Verbosity filtering → DslParser → outbound logging
    /// → IM Adapter rendering).
    Reply(Vec<ContentBlock>),
    /// Trigger manual compaction (Gateway delegates to session_handler).
    TriggerCompact { instruction: Option<String> },
    /// No action needed.
    Nothing,
}

/// Execution context for [`SlashResult::execute`](super::SlashResult::execute).
///
/// Encapsulates the session reference, channel identity, and reply channel so
/// that each `SlashResult` variant can complete its side effects internally.
pub struct SideEffectContext {
    /// The current session ID.
    pub session_id: String,
    /// The channel identifier (e.g. `"feishu"`).
    pub channel: String,
    /// Session manager for accessing conversation sessions.
    pub session_manager: Arc<SessionManager>,
    /// Channel to send reply actions back to the Gateway.
    reply_tx: tokio::sync::mpsc::Sender<ReplyAction>,
}

impl SideEffectContext {
    pub fn new(
        session_id: String,
        channel: String,
        session_manager: Arc<SessionManager>,
        reply_tx: tokio::sync::mpsc::Sender<ReplyAction>,
    ) -> Self {
        Self {
            session_id,
            channel,
            session_manager,
            reply_tx,
        }
    }

    /// Send a content-block reply to the user.
    ///
    /// The reply is wrapped in [`ReplyAction::Reply`] and delivered through
    /// the outbound Processor Chain (Verbosity filtering → DslParser →
    /// outbound logging → IM Adapter rendering).
    pub async fn reply(&self, blocks: Vec<ContentBlock>) {
        let _ = self.reply_tx.send(ReplyAction::Reply(blocks)).await;
    }

    /// Trigger manual compaction.
    pub async fn trigger_compact(&self, instruction: Option<String>) {
        let _ = self
            .reply_tx
            .send(ReplyAction::TriggerCompact { instruction })
            .await;
    }

    /// Get the conversation session for the current session ID.
    pub async fn get_conversation_session(
        &self,
    ) -> Option<Arc<tokio::sync::RwLock<closeclaw_llm::session::ConversationSession>>> {
        self.session_manager
            .get_conversation_session(&self.session_id)
            .await
    }
}

/// Execute side effects for [`SlashResult::SystemAppend`](super::SlashResult::SystemAppend).
pub(crate) async fn execute_system_append(
    ctx: &SideEffectContext,
    action: &super::handler::SystemAppendAction,
) {
    use super::handler::SystemAppendAction;
    match action {
        SystemAppendAction::Add(content) => {
            if let Some(cs) = ctx.get_conversation_session().await {
                let mut session = cs.write().await;
                let index = session.add_system_append(content.clone());
                ctx.reply(vec![ContentBlock::Text(format!(
                    "已追加指令（序号 {index}）"
                ))])
                .await;
            } else {
                ctx.reply(vec![ContentBlock::Text(
                    "当前会话未激活，无法追加指令".to_owned(),
                )])
                .await;
            }
        }
        SystemAppendAction::Clear => {
            if let Some(cs) = ctx.get_conversation_session().await {
                let mut session = cs.write().await;
                let count = session.clear_system_appends();
                ctx.reply(vec![ContentBlock::Text(format!(
                    "已清除 {count} 条追加指令"
                ))])
                .await;
            } else {
                ctx.reply(vec![ContentBlock::Text(
                    "当前会话未激活，无法清除指令".to_owned(),
                )])
                .await;
            }
        }
    }
}

/// Execute side effects for [`SlashResult::NewSession`](super::SlashResult::NewSession).
pub(crate) async fn execute_new_session(ctx: &SideEffectContext) {
    let agent_id = ctx
        .session_manager
        .get_chat_id(&ctx.session_id)
        .await
        .unwrap_or_default();
    let new_session_id = ctx
        .session_manager
        .force_new_for_channel(&ctx.channel, &agent_id)
        .await;
    ctx.reply(vec![ContentBlock::Text(format!(
        "已创建新 session：{new_session_id}"
    ))])
    .await;
}

/// Execute side effects for [`SlashResult::Stop`](super::SlashResult::Stop).
pub(crate) async fn execute_stop(ctx: &SideEffectContext) {
    let Some(conv) = ctx
        .session_manager
        .get_conversation_session(&ctx.session_id)
        .await
    else {
        ctx.reply(vec![ContentBlock::Text("当前会话未激活".to_owned())])
            .await;
        return;
    };
    let busy = {
        let cs = conv.read().await;
        cs.is_llm_busy()
    };
    if busy {
        let mut cs = conv.write().await;
        cs.cancel_token.cancel();
        let handles_to_stop: Vec<_> = {
            let child_handles = cs
                .child_handles
                .read()
                .expect("child_handles lock poisoned");
            child_handles.values().filter_map(|w| w.upgrade()).collect()
        };
        cs.clear_pending();
        drop(cs);
        for child in handles_to_stop {
            let child_cs = child.read().await;
            child_cs.cancel_token.cancel();
        }
    }
    ctx.reply(vec![ContentBlock::Text("已停止当前任务".to_owned())])
        .await;
}
