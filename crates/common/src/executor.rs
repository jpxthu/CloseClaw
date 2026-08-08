//! Slash command executor types and traits.
//!
//! Defines [`SlashResultExecutor`], an extension trait on
//! [`SlashResult`] that performs the actual side-effect dispatch
//! through a [`SideEffectContext`]. The concrete [`SlashEffectExecutor`]
//! implementation lives in the `gateway` crate, which owns the
//! session and permission capabilities.
//!
//! Note: These types live in `closeclaw-common` (not `closeclaw-slash`)
//! because the `closeclaw-gateway` crate cannot depend on `closeclaw-slash`
//! (cycle: gateway → slash → tools → gateway). Both gateway and slash
//! import from here.

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::processor::ContentBlock;
use crate::session_lookup::{PendingMessage, SessionLookup};
use crate::slash_router::{SlashResult, SystemAppendAction};
use crate::{ReasoningLevel, VerbosityLevel};

// ── Compaction types ──────────────────────────────────────────────────

/// Result of a compaction operation.
#[derive(Debug, Clone)]
pub struct CompactionResult {
    /// Whether compaction was performed.
    pub performed: bool,
    /// Number of tokens in the original session.
    pub original_tokens: usize,
    /// Number of tokens after compaction (meaningful only if performed=true).
    pub compacted_tokens: usize,
    /// Human-readable message describing the outcome.
    pub message: String,
    /// Character count before compaction.
    pub before_char_count: usize,
    /// Character count after compaction.
    pub after_char_count: usize,
    /// Token count before compaction.
    pub before_token_count: usize,
    /// Token count after compaction.
    pub after_token_count: usize,
    /// Boundary system message containing the summary.
    pub boundary_message: String,
    /// Whether this compaction was triggered automatically.
    pub is_auto: bool,
}

/// Errors that can occur during compaction.
#[derive(Debug, thiserror::Error)]
pub enum CompactionError {
    /// LLM call failed.
    #[error("LLM call failed: {0}")]
    LLMCallFailed(String),

    /// Session not found.
    #[error("session not found: {0}")]
    SessionNotFound(String),

    /// Failed to parse summary from LLM response.
    #[error("Failed to parse summary from LLM response")]
    SummaryParseFailed,

    /// No messages provided for compaction.
    #[error("No messages provided for compaction")]
    EmptyMessages,

    /// Required handler not available.
    #[error("handler not available: {0}")]
    HandlerNotAvailable(String),
}

// ── Executor types ────────────────────────────────────────────────────

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
    async fn execute_stop(&self, session_id: &str, cascade: bool, force: bool);

    /// Create a new session for the given channel.
    ///
    /// Returns the new session_id.
    async fn execute_new_session(&self, session_id: &str, channel: &str) -> String;

    /// Trigger context compaction with an optional custom instruction.
    async fn execute_compact(
        &self,
        session_id: &str,
        instruction: Option<String>,
    ) -> Result<CompactionResult, CompactionError>;

    /// Apply a system prompt append/clear action.
    ///
    /// Returns the relevant count: for `Add`, the 1-based index of the
    /// newly appended item; for `Clear`, the number of items cleared.
    async fn execute_system_append(&self, session_id: &str, action: &SystemAppendAction) -> usize;

    /// Set the reasoning level for the session.
    async fn execute_set_reasoning(&self, session_id: &str, level: ReasoningLevel);

    /// Set the verbosity level for the session.
    async fn execute_set_verbosity(&self, session_id: &str, level: VerbosityLevel);

    /// Set the session mode for the session.
    async fn execute_set_mode(&self, session_id: &str, mode: &str);

    /// Execute a shell command for the given agent.
    ///
    /// The implementation runs the command and returns output as
    /// `ContentBlock::Text`. Permission is evaluated at the Gateway layer
    /// (check_slash_permission) before the executor is invoked.
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
/// Implemented for [`SlashResult`]. The gateway
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

/// Send a text reply through the context channel.
async fn send_reply(ctx: &SideEffectContext, text: String) {
    let _ = ctx
        .reply_tx
        .send(ReplyAction::Reply(vec![ContentBlock::Text(text)]))
        .await;
}

/// Handle `SlashResult::Reply` — send a text block to the user.
async fn execute_reply(ctx: &SideEffectContext, text: String) {
    send_reply(ctx, text).await;
}

/// Handle `SlashResult::SetMode` — set session mode and optionally inject
/// an initial user message.
async fn execute_set_mode(
    ctx: &SideEffectContext,
    mode: String,
    initial_input: Option<String>,
    reply_message: Option<String>,
) {
    ctx.executor.execute_set_mode(&ctx.session_id, &mode).await;
    let reply = reply_message.unwrap_or_else(|| format!("Mode set to: {mode}"));
    send_reply(ctx, reply).await;

    if let Some(input) = initial_input {
        let pending_msg = PendingMessage::with_role(
            format!("slash-initial-{}", chrono::Utc::now().timestamp_millis()),
            input,
            "user".to_string(),
        );
        if let Err(e) = ctx
            .session_manager
            .push_pending_message(&ctx.session_id, pending_msg)
            .await
        {
            tracing::warn!(
                session_id = %ctx.session_id,
                error = %e,
                "failed to push initial_input pending message"
            );
        }
    }
}

/// Handle `SlashResult::NewSession` — create a fresh session and reply.
async fn execute_new_session(ctx: &SideEffectContext) {
    let new_id = ctx
        .executor
        .execute_new_session(&ctx.session_id, &ctx.channel)
        .await;
    send_reply(ctx, format!("已创建新 session：{new_id}")).await;
}

/// Handle `SlashResult::Stop` — stop the current LLM turn.
async fn execute_stop(ctx: &SideEffectContext, cascade: bool, force: bool) {
    ctx.executor
        .execute_stop(&ctx.session_id, cascade, force)
        .await;
    send_reply(ctx, "已停止当前任务".into()).await;
}

/// Handle `SlashResult::Compact` — trigger context compaction.
async fn execute_compact(ctx: &SideEffectContext, instruction: Option<String>) {
    let reply = match ctx
        .executor
        .execute_compact(&ctx.session_id, instruction)
        .await
    {
        Ok(r) => r.message,
        Err(e) => format!("Compact failed: {e}"),
    };
    send_reply(ctx, reply).await;
}

/// Handle `SlashResult::SystemAppend` — add or clear system prompt instructions.
async fn execute_system_append(ctx: &SideEffectContext, action: SystemAppendAction) {
    let count = ctx
        .executor
        .execute_system_append(&ctx.session_id, &action)
        .await;
    match action {
        SystemAppendAction::Add(_) => {
            send_reply(ctx, format!("已追加指令 #{count}")).await;
        }
        SystemAppendAction::Clear => {
            send_reply(ctx, format!("已清除 {count} 条追加指令")).await;
        }
    }
}

/// Handle `SlashResult::Exec` — run a shell command.
async fn execute_exec(ctx: &SideEffectContext, command: String) {
    let agent_id = ctx
        .session_manager
        .get_chat_id(&ctx.session_id)
        .await
        .unwrap_or_default();
    let blocks = ctx
        .executor
        .execute_exec(&ctx.session_id, &agent_id, &command)
        .await;
    let _ = ctx.reply_tx.send(ReplyAction::Reply(blocks)).await;
}

/// Handle `SlashResult::SetReasoning` — set reasoning depth.
async fn execute_set_reasoning(ctx: &SideEffectContext, level: ReasoningLevel) {
    ctx.executor
        .execute_set_reasoning(&ctx.session_id, level)
        .await;
    send_reply(ctx, format!("推理深度已设为 {level}")).await;
}

/// Handle `SlashResult::SetVerbosity` — set output verbosity.
async fn execute_set_verbosity(ctx: &SideEffectContext, level: VerbosityLevel) {
    ctx.executor
        .execute_set_verbosity(&ctx.session_id, level)
        .await;
    send_reply(ctx, format!("输出详细度已设置为 {level}")).await;
}

/// Handle `SlashResult::InjectMeta` — load a skill via system append.
async fn execute_inject_meta(ctx: &SideEffectContext, content: String) {
    ctx.executor
        .execute_system_append(&ctx.session_id, &SystemAppendAction::Add(content))
        .await;
    send_reply(ctx, "技能已加载".into()).await;
}

/// Handle `SlashResult::Unknown` — report an unrecognized command.
async fn execute_unknown(ctx: &SideEffectContext, cmd: String) {
    send_reply(ctx, format!("Unknown command: /{cmd}")).await;
}

#[async_trait]
impl SlashResultExecutor for SlashResult {
    async fn execute(self, ctx: &SideEffectContext) {
        match self {
            SlashResult::Reply(text) => execute_reply(ctx, text).await,
            SlashResult::SetMode {
                mode,
                plan_file_path: _,
                initial_input,
                reply_message,
            } => execute_set_mode(ctx, mode, initial_input, reply_message).await,
            SlashResult::NewSession => execute_new_session(ctx).await,
            SlashResult::Stop { cascade, force } => execute_stop(ctx, cascade, force).await,
            SlashResult::Compact { instruction } => execute_compact(ctx, instruction).await,
            SlashResult::SystemAppend { action } => execute_system_append(ctx, action).await,
            SlashResult::Exec {
                command,
                requires_permission: _,
            } => execute_exec(ctx, command).await,
            SlashResult::SetReasoning { level } => execute_set_reasoning(ctx, level).await,
            SlashResult::SetVerbosity { level } => execute_set_verbosity(ctx, level).await,
            SlashResult::InjectMeta { content } => execute_inject_meta(ctx, content).await,
            SlashResult::Unknown(cmd) => execute_unknown(ctx, cmd).await,
            // PermissionOp is intercepted in execute_and_route before execute()
            // is called. This arm exists for exhaustive match compilation.
            SlashResult::PermissionOp { .. } => {}
            // UserApprove/UserReject are intercepted in execute_and_route before
            // execute() is called.
            SlashResult::UserApprove { .. } | SlashResult::UserReject { .. } => {}
        }
    }
}
