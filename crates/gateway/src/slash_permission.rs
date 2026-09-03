//! Slash command permission control for the Gateway.
//!
//! Provides `set_slash_dispatcher()`, `set_permission_engine()`, and
//! `dispatch_slash()` for routing slash commands through the permission
//! engine before execution.

use std::sync::Arc;

use closeclaw_common::executor::{
    ReplyAction, SideEffectContext, SlashEffectExecutor, SlashResultExecutor,
};
use closeclaw_common::processor::ContentBlock;
use closeclaw_common::slash_router::{
    SlashContext, SlashHandler, SlashResult, SlashRouter, SystemAppendAction,
};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_types::{
    Caller, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
use closeclaw_session::persistence::PendingMessage;

use super::{Gateway, HandleResult, SessionManager, SessionMessageHandler};

/// Parse a slash command from raw content.
///
/// Returns `Some((command, args))` where `command` is without the
/// leading `/` and `args` is the remainder. Returns `None` if the
/// content does not start with `/`.
fn parse_slash(content: &str) -> Option<(&str, &str)> {
    let trimmed = content.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let without_slash = &trimmed[1..];
    let (cmd, args) = without_slash
        .split_once(char::is_whitespace)
        .unwrap_or((without_slash, ""));
    if cmd.is_empty() {
        return None;
    }
    Some((cmd, args.trim_start()))
}

/// Routing context for slash command dispatch.
///
/// Bundles the four routing fields (`session_id`, `sender_id`,
/// `channel`, `peer_id`) so they can be threaded through the
/// permission-checking methods without exceeding the 6-parameter
/// hard limit.
pub(crate) struct SlashRouteCtx<'a> {
    /// Session identifier for the current dispatch.
    pub(crate) session_id: &'a str,
    /// Sender user identifier (None when unavailable).
    pub(crate) sender_id: Option<&'a str>,
    /// IM channel identifier (e.g. "feishu", "telegram").
    pub(crate) channel: &'a str,
    /// IM peer/chat identifier for outbound replies.
    pub(crate) peer_id: &'a str,
}

impl Gateway {
    /// Install the slash command dispatcher.
    pub async fn set_slash_dispatcher(&self, dispatcher: Arc<dyn SlashRouter>) {
        let mut slot = self.slash_dispatcher.write().await;
        *slot = Some(dispatcher);
    }

    /// Install the permission engine (used for slash command authorization).
    pub async fn set_permission_engine(&self, engine: Arc<tokio::sync::RwLock<PermissionEngine>>) {
        let mut slot = self.permission_engine.write().await;
        *slot = Some(engine);
    }

    /// Reply with an "unknown command" message.
    async fn reply_unknown_cmd(&self, cmd: &str, session_id: &str, channel: &str) {
        let reply = format!("未知指令：/{cmd}。输入 /help 查看所有可用指令。");
        self.route_slash_reply(session_id, channel, vec![ContentBlock::Text(reply)])
            .await;
    }

    /// Enqueue a non-immediate slash command when the session is busy.
    async fn enqueue_pending_slash(
        &self,
        session_id: &str,
        content: &str,
        peer_id: &str,
        channel: &str,
    ) {
        let msg = PendingMessage::with_role(
            format!("pending-{}", chrono::Utc::now().timestamp_millis()),
            content.to_owned(),
            "user".to_string(),
        );
        if let Err(e) = self
            .session_manager
            .push_pending_message(session_id, msg)
            .await
        {
            tracing::warn!(
                session_id,
                error = %e,
                "failed to enqueue pending slash command"
            );
        }
        let text = closeclaw_session::notifications::QUEUE_NOTIFICATION_TEXT;
        self.send_system_notification(peer_id, channel, text).await;
    }

    /// Dispatch a slash command with permission checks.
    ///
    /// Returns `Some(HandleResult::SlashHandled)` when the message is consumed
    /// as a slash command (including permission-denied replies), or `None` if
    /// the message is not a recognized slash command and should fall through
    /// to the normal session handler.
    ///
    /// Three-branch permission routing (恢复自 PR #811 之前的语义):
    /// 1. `sender_id == Some("owner")` → 直接分派 handler（Owner 短路）
    /// 2. `handler.requires_permission() == true` → handler.handle() 执行后，
    ///    调用 `permission_engine.evaluate()`；返回 `Denied` 时回复"权限不足"
    ///    并跳过 SlashResult.execute()
    /// 3. `handler.requires_permission() == false` → 直接分派 handler 并执行
    ///
    /// `channel` 会被填入 `SlashContext.channel`，让 handler 知晓入站消息来自哪个
    /// channel（如 "feishu"）。
    pub(crate) async fn dispatch_slash(
        &self,
        session_id: &str,
        content: &str,
        sender_id: Option<&str>,
        channel: &str,
        peer_id: Option<&str>,
    ) -> Option<HandleResult> {
        let dispatcher_guard = self.slash_dispatcher.read().await;
        let dispatcher = dispatcher_guard.as_ref()?;

        let (cmd, args) = match parse_slash(content) {
            Some(parsed) => parsed,
            None => return None,
        };

        let Some(handler) = dispatcher.get_handler(cmd) else {
            self.reply_unknown_cmd(cmd, session_id, channel).await;
            return Some(HandleResult::SlashHandled);
        };

        // Non-immediate commands: if session is busy, enqueue for later.
        if !dispatcher.is_immediate(content)
            && self.session_manager.is_session_busy(session_id).await
        {
            self.enqueue_pending_slash(session_id, content, peer_id.unwrap_or(""), channel)
                .await;
            return Some(HandleResult::SlashHandled);
        }

        let route_ctx = SlashRouteCtx {
            session_id,
            sender_id,
            channel,
            peer_id: peer_id.unwrap_or(""),
        };
        self.execute_and_route(handler.as_ref(), cmd, args, &route_ctx)
            .await
    }

    /// Build a [`PermissionRequest`] for the given slash command.
    ///
    /// Resolves the agent ID from the session and constructs a
    /// [`PermissionRequest::WithCaller`] suitable for the permission engine.
    async fn build_permission_request(
        &self,
        cmd: &str,
        sender_id: Option<&str>,
        session_id: &str,
    ) -> PermissionRequest {
        let agent_id = self
            .session_manager
            .get_chat_id(session_id)
            .await
            .unwrap_or_default();

        let caller = Caller {
            user_id: sender_id.unwrap_or("").to_owned(),
            agent: agent_id.clone(),
        };
        PermissionRequest::WithCaller {
            caller,
            request: PermissionRequestBody::SlashCommand {
                agent: agent_id,
                command: cmd.to_owned(),
            },
        }
    }

    /// Permission engine check: Owner short-circuit + engine evaluation.
    ///
    /// Checks whether a slash command should be allowed based solely on
    /// the permission engine, without consulting the handler's
    /// `requires_permission()`. Used directly by `execute_and_route` for
    /// `SlashResult::Exec { requires_permission: true }` to bypass
    /// handler-level checks.
    async fn check_engine_permission(&self, cmd: &str, ctx: &SlashRouteCtx<'_>) -> bool {
        if ctx.sender_id == Some("owner") {
            return true;
        }

        let engine_guard = self.permission_engine.read().await;
        let Some(engine) = engine_guard.as_ref() else {
            tracing::warn!(
                cmd,
                session_id = %ctx.session_id,
                channel = %ctx.channel,
                "permission engine not configured; denying slash command"
            );
            if let Err(e) = self
                .send_outbound_simplified(ctx.peer_id, ctx.channel, "权限不足：权限引擎未配置")
                .await
            {
                tracing::warn!(
                    cmd,
                    session_id = %ctx.session_id,
                    channel = %ctx.channel,
                    error = %e,
                    "failed to send permission engine \
                     not configured notification"
                );
            }
            return false;
        };

        let request = self
            .build_permission_request(cmd, ctx.sender_id, ctx.session_id)
            .await;

        // Gateway-level permission check: evaluate current agent config
        // and user permissions without traversing the agent spawn chain.
        // SlashCommand is handled independently by the Gateway layer
        // (three-branch routing) and does not involve agent inheritance.
        let response = engine.read().await.evaluate(request, None);
        if let PermissionResponse::Denied { reason, .. } = response {
            if let Err(e) = self
                .send_outbound_simplified(ctx.peer_id, ctx.channel, &format!("权限不足：{reason}"))
                .await
            {
                tracing::warn!(
                    cmd,
                    session_id = %ctx.session_id,
                    channel = %ctx.channel,
                    error = %e,
                    "failed to send permission denied \
                     notification"
                );
            }
            return false;
        }
        true
    }

    /// Three-branch permission check. Returns `true` if the command may
    /// proceed, `false` if it was denied (reply already sent).
    async fn check_slash_permission(&self, cmd: &str, ctx: &SlashRouteCtx<'_>) -> bool {
        // Branch 1: Owner 短路
        if ctx.sender_id == Some("owner") {
            return true;
        }

        // Branch 3: 普通指令直通
        let dispatcher_guard = self.slash_dispatcher.read().await;
        let dispatcher = match dispatcher_guard.as_ref() {
            Some(d) => d,
            None => return true,
        };
        let Some(handler) = dispatcher.get_handler(cmd) else {
            return true;
        };
        if !handler.requires_permission() {
            return true;
        }
        drop(dispatcher_guard);

        // Branch 2: 高危指令走权限引擎
        self.check_engine_permission(cmd, ctx).await
    }

    /// Route a slash reply through the outbound Processor Chain.
    ///
    /// ContentBlock[] from [`ReplyAction::Reply`] is sent through the same
    /// outbound pipeline as LLM responses: Verbosity filtering → DslParser →
    /// outbound logging → IM Adapter rendering.
    ///
    /// Falls back to plain-text `send_reply` when the outbound chain is
    /// unavailable (e.g. no plugin registered in tests).
    async fn route_slash_reply(&self, session_id: &str, channel: &str, blocks: Vec<ContentBlock>) {
        let raw_output = blocks
            .iter()
            .find_map(|b| match b {
                ContentBlock::Text(t) => Some(t.clone()),
                _ => None,
            })
            .unwrap_or_default();
        if let Err(e) = self
            .send_outbound(session_id, channel, &raw_output, blocks, None, None)
            .await
        {
            tracing::debug!(
                session_id,
                channel,
                error = %e,
                "slash reply outbound failed, falling back to send_reply"
            );
            if let Some(sh) = self.session_handler.get() {
                sh.send_reply(raw_output).await;
            }
        }
    }

    /// Invoke the handler with a constructed `SlashContext`, then route the
    /// returned `SlashResult` to the appropriate side effect.
    ///
    /// Constructs a [`SideEffectContext`] with a [`GatewaySlashExecutor`]
    /// and calls [`SlashResult::execute`], then dispatches the produced
    /// [`ReplyAction`]s through the session handler.
    /// Permission check for `execute_and_route`: dispatches to the
    /// appropriate engine/handler check based on the [`SlashResult`] variant.
    ///
    /// Returns `true` when the command is allowed, `false` when denied
    /// (reply already sent).
    async fn check_permission_for_execute(
        &self,
        result: &SlashResult,
        cmd_name: &str,
        ctx: &SlashRouteCtx<'_>,
    ) -> bool {
        match result {
            SlashResult::Exec {
                requires_permission: false,
                ..
            } => true,
            SlashResult::Exec {
                requires_permission: true,
                ..
            } => self.check_engine_permission(cmd_name, ctx).await,
            _ => self.check_slash_permission(cmd_name, ctx).await,
        }
    }

    /// Execute a [`SlashResult`]'s side effects and route replies back
    /// through the outbound processor chain.
    async fn execute_side_effects(&self, result: SlashResult, session_id: &str, channel: &str) {
        let (reply_tx, mut reply_rx) = tokio::sync::mpsc::channel(8);
        let session_mgr: Arc<dyn closeclaw_common::SessionLookup> =
            self.session_manager.clone() as Arc<dyn closeclaw_common::SessionLookup>;
        let executor: Arc<dyn SlashEffectExecutor> = Arc::new(GatewaySlashExecutor {
            session_manager: Arc::clone(&self.session_manager),
            session_handler: self.session_handler.get().cloned(),
        });
        let side_effect_ctx = SideEffectContext {
            session_id: session_id.to_owned(),
            channel: channel.to_owned(),
            session_manager: session_mgr,
            reply_tx,
            executor,
        };

        result.execute(&side_effect_ctx).await;
        drop(side_effect_ctx);

        while let Some(action) = reply_rx.recv().await {
            match action {
                ReplyAction::Reply(blocks) => {
                    self.route_slash_reply(session_id, channel, blocks).await;
                }
                ReplyAction::TriggerCompact { .. } => {
                    // Compact is already handled by the executor; no-op.
                }
                ReplyAction::Nothing => {}
            }
        }
    }

    async fn execute_and_route(
        &self,
        handler: &dyn SlashHandler,
        cmd_name: &str,
        args: &str,
        ctx: &SlashRouteCtx<'_>,
    ) -> Option<HandleResult> {
        let slash_ctx = SlashContext {
            command: cmd_name.to_owned(),
            sender_id: ctx.sender_id.unwrap_or("").to_owned(),
            session_id: ctx.session_id.to_owned(),
            channel: ctx.channel.to_owned(),
        };
        let result = handler.handle(args, &slash_ctx).await;

        // Permission check AFTER handler returns SlashResult but BEFORE execute.
        if !self
            .check_permission_for_execute(&result, cmd_name, ctx)
            .await
        {
            return Some(HandleResult::SlashHandled);
        }

        self.execute_side_effects(result, ctx.session_id, ctx.channel)
            .await;
        Some(HandleResult::SlashHandled)
    }
}

// ── SlashEffectExecutor implementation ──────────────────────────────────

/// Gateway-side implementation of [`SlashEffectExecutor`].
///
/// Bridges the common trait to the Gateway's concrete
/// `SessionManager` and `SessionMessageHandler` for performing
/// slash command side effects.
pub(crate) struct GatewaySlashExecutor {
    session_manager: Arc<SessionManager>,
    session_handler: Option<Arc<SessionMessageHandler>>,
}

// ── GatewaySlashExecutor helper functions ───────────────────────────────
// Imported from slash_executor_helpers.rs to keep this file under the
// 800-line soft limit.
use crate::slash_executor_helpers::*;

// ── SlashEffectExecutor implementation ──────────────────────────────────

#[async_trait::async_trait]
impl SlashEffectExecutor for GatewaySlashExecutor {
    async fn execute_stop(&self, session_id: &str, cascade: bool, force: bool) {
        gw_stop(&self.session_manager, session_id, cascade, force).await;
    }

    async fn execute_new_session(&self, _session_id: &str, channel: &str) -> String {
        gw_new_session(&self.session_manager, _session_id, channel).await
    }

    async fn execute_compact(
        &self,
        session_id: &str,
        instruction: Option<String>,
    ) -> Result<
        closeclaw_session::compaction::CompactionResult,
        closeclaw_session::compaction::CompactionError,
    > {
        let sh = self.session_handler.clone();
        gw_compact(&self.session_manager, &sh, session_id, instruction).await
    }

    async fn execute_system_append(&self, session_id: &str, action: &SystemAppendAction) -> usize {
        let sh = self.session_handler.clone();
        gw_system_append(&self.session_manager, &sh, session_id, action).await
    }

    async fn execute_set_reasoning(
        &self,
        session_id: &str,
        level: closeclaw_session::persistence::ReasoningLevel,
    ) {
        let sh = self.session_handler.clone();
        if let Some(cs) = gw_get_cs_or_reply(
            &self.session_manager,
            &sh,
            session_id,
            "session 不存在，无法设置推理深度",
        )
        .await
        {
            cs.write().await.set_reasoning_level(level);
        }
    }

    async fn execute_set_verbosity(
        &self,
        session_id: &str,
        level: closeclaw_common::VerbosityLevel,
    ) {
        let sh = self.session_handler.clone();
        if let Some(cs) = gw_get_cs_or_reply(
            &self.session_manager,
            &sh,
            session_id,
            "session 不存在，无法设置输出详细度",
        )
        .await
        {
            cs.write().await.set_verbosity_level(level);
        }
    }

    async fn execute_set_mode(&self, session_id: &str, mode: &str) {
        let sh = self.session_handler.clone();
        if let Some(cs) = gw_get_cs_or_reply(
            &self.session_manager,
            &sh,
            session_id,
            "session 不存在，无法设置 mode",
        )
        .await
        {
            match closeclaw_common::SessionMode::from_str_opt(mode) {
                Some(parsed) => cs.read().await.set_pending_session_mode(parsed),
                None => tracing::warn!(
                    session_id,
                    mode,
                    "unknown session mode; keeping current mode"
                ),
            }
        }
    }

    async fn execute_exec(
        &self,
        _session_id: &str,
        _agent_id: &str,
        command: &str,
    ) -> Vec<ContentBlock> {
        gw_exec(command).await
    }
}

// ── GatewaySlashExecutor inherent methods ──────────────────────────────

impl GatewaySlashExecutor {
    /// Create a new executor for testing purposes.
    #[allow(dead_code)]
    pub(crate) fn new(
        session_manager: Arc<SessionManager>,
        session_handler: Option<Arc<SessionMessageHandler>>,
    ) -> Self {
        Self {
            session_manager,
            session_handler,
        }
    }
}
