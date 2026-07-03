//! Slash command permission control for the Gateway.
//!
//! Provides `set_slash_dispatcher()`, `set_permission_engine()`, and
//! `dispatch_slash()` for routing slash commands through the permission
//! engine before execution.

use std::sync::Arc;

use super::slash_execute::execute_slash_result;
use closeclaw_common::processor::ContentBlock;
use closeclaw_common::slash_router::{
    ReplyAction, SideEffectContext, SlashContext, SlashEffectExecutor, SlashHandler, SlashRouter,
    SystemAppendAction,
};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_types::{
    Caller, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
use closeclaw_session::persistence::PendingMessage;

use super::{Gateway, HandleResult, SessionManager, SessionMessageHandler};
use closeclaw_llm::session::ConversationSession;
use tokio::sync::RwLock;

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

impl Gateway {
    /// Install the slash command dispatcher.
    pub async fn set_slash_dispatcher(&self, dispatcher: Arc<dyn SlashRouter>) {
        let mut slot = self.slash_dispatcher.write().await;
        *slot = Some(dispatcher);
    }

    /// Install the permission engine (used for slash command authorization).
    pub async fn set_permission_engine(&self, engine: Arc<PermissionEngine>) {
        let mut slot = self.permission_engine.write().await;
        *slot = Some(engine);
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
    ///    调用 `permission_engine.evaluate()`；返回 `Denied` 时回复"无权限"
    ///    并跳过 SlashResult.execute()
    /// 3. `handler.requires_permission() == false` → 直接分派 handler 并执行
    ///
    /// `channel` 会被填入 `SlashContext.channel`，让 handler 知晓入站消息来自哪个
    /// channel（如 "feishu"）。
    pub(super) async fn dispatch_slash(
        &self,
        session_id: &str,
        content: &str,
        sender_id: Option<&str>,
        channel: &str,
    ) -> Option<HandleResult> {
        let dispatcher_guard = self.slash_dispatcher.read().await;
        let dispatcher = dispatcher_guard.as_ref()?;

        let (cmd, args) = match parse_slash(content) {
            Some(parsed) => parsed,
            None => return None,
        };
        let Some(handler) = dispatcher.get_handler(cmd) else {
            let reply = format!("未知指令：/{cmd}。输入 /help 查看所有可用指令。");
            if let Some(sh) = self.session_handler.as_ref() {
                sh.send_reply(reply).await;
            }
            return Some(HandleResult::SlashHandled);
        };

        // Non-immediate commands: if session is busy, enqueue for later.
        if !dispatcher.is_immediate(cmd) && self.session_manager.is_session_busy(session_id).await {
            let msg = PendingMessage::new(
                format!("pending-{}", chrono::Utc::now().timestamp_millis()),
                content.to_owned(),
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
            if let Some(sh) = self.session_handler.as_ref() {
                sh.send_reply("⏳ 正在排队...".to_owned()).await;
            }
            return Some(HandleResult::SlashHandled);
        }

        self.execute_and_route(handler.as_ref(), cmd, args, session_id, sender_id, channel)
            .await
    }

    /// Three-branch permission check. Returns `true` if the command may
    /// proceed, `false` if it was denied (reply already sent).
    async fn check_slash_permission(
        &self,
        cmd: &str,
        sender_id: Option<&str>,
        session_id: &str,
    ) -> bool {
        // Branch 1: Owner 短路
        if sender_id == Some("owner") {
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
        let engine_guard = self.permission_engine.read().await;
        let Some(engine) = engine_guard.as_ref() else {
            tracing::warn!(
                cmd,
                "permission engine not configured; denying high-risk slash command"
            );
            self.send_reply_if_available("无权限：权限引擎未配置").await;
            return false;
        };

        let agent_id = self
            .session_manager
            .get_chat_id(session_id)
            .await
            .unwrap_or_default();

        // Build agent_permissions map from config_manager for chain intersection.
        let agent_permissions = match self.session_manager.get_config_manager().await {
            Some(cm) => cm.agent_permissions(),
            None => std::collections::HashMap::new(),
        };

        let caller = Caller {
            user_id: sender_id.unwrap_or("").to_owned(),
            agent: agent_id.clone(),
            creator_id: String::new(),
        };
        let request = PermissionRequest::WithCaller {
            caller,
            request: PermissionRequestBody::SlashCommand {
                agent: agent_id,
                command: cmd.to_owned(),
            },
        };

        // Chain-aware permission check: dimension-level intersection
        // with the parent agent chain.
        let response = engine
            .evaluate_with_chain(
                request,
                &*self.session_manager,
                session_id,
                &agent_permissions,
            )
            .await;
        if let PermissionResponse::Denied { reason, .. } = response {
            self.send_reply_if_available(&format!("无权限：{reason}"))
                .await;
            return false;
        }
        true
    }

    async fn send_reply_if_available(&self, text: &str) {
        if let Some(sh) = self.session_handler.as_ref() {
            sh.send_reply(text.to_owned()).await;
        }
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
            .send_outbound(session_id, channel, &raw_output, blocks)
            .await
        {
            tracing::debug!(
                session_id,
                channel,
                error = %e,
                "slash reply outbound failed, falling back to send_reply"
            );
            if let Some(sh) = self.session_handler.as_ref() {
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
    async fn execute_and_route(
        &self,
        handler: &dyn SlashHandler,
        cmd_name: &str,
        args: &str,
        session_id: &str,
        sender_id: Option<&str>,
        channel: &str,
    ) -> Option<HandleResult> {
        let slash_ctx = SlashContext {
            command: cmd_name.to_owned(),
            sender_id: sender_id.unwrap_or("").to_owned(),
            session_id: session_id.to_owned(),
            channel: channel.to_owned(),
        };
        let result = handler.handle(args, &slash_ctx).await;

        // Permission check: after handler, before execute (design doc alignment).
        if !self
            .check_slash_permission(cmd_name, sender_id, session_id)
            .await
        {
            return Some(HandleResult::SlashHandled);
        }

        let (reply_tx, mut reply_rx) = tokio::sync::mpsc::channel(8);
        let session_mgr: Arc<dyn closeclaw_common::SessionLookup> =
            self.session_manager.clone() as Arc<dyn closeclaw_common::SessionLookup>;
        let perm_engine = self.permission_engine.read().await.clone();
        let executor: Arc<dyn SlashEffectExecutor> = Arc::new(GatewaySlashExecutor {
            session_manager: Arc::clone(&self.session_manager),
            session_handler: self.session_handler.clone(),
            permission_engine: perm_engine,
        });
        let side_effect_ctx = SideEffectContext {
            session_id: session_id.to_owned(),
            channel: channel.to_owned(),
            session_manager: session_mgr,
            reply_tx,
            executor,
        };

        execute_slash_result(&result, &side_effect_ctx).await;
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

        Some(HandleResult::SlashHandled)
    }
}

// ── SlashEffectExecutor implementation ──────────────────────────────────

/// Gateway-side implementation of [`SlashEffectExecutor`].
///
/// Bridges the common trait to the Gateway's concrete
/// `SessionManager` and `SessionMessageHandler` for performing
/// slash command side effects.
struct GatewaySlashExecutor {
    session_manager: Arc<SessionManager>,
    session_handler: Option<Arc<SessionMessageHandler>>,
    permission_engine: Option<Arc<PermissionEngine>>,
}

#[async_trait::async_trait]
impl SlashEffectExecutor for GatewaySlashExecutor {
    async fn execute_stop(&self, session_id: &str) {
        let cs: Option<Arc<RwLock<ConversationSession>>> = self
            .session_manager
            .get_conversation_session(session_id)
            .await;
        if let Some(cs) = cs {
            cs.read().await.stop(false).await;
        } else if let Some(sh) = self.session_handler.as_ref() {
            sh.send_reply("session 不存在，无法停止".to_owned()).await;
        }
    }

    async fn execute_new_session(&self, _session_id: &str, channel: &str) {
        // force_new_for_channel creates a fresh session for the channel and
        // updates the channel→session mapping so subsequent messages route to it.
        let agent_id = self
            .session_manager
            .get_chat_id(_session_id)
            .await
            .unwrap_or_default();
        let _new_id = self
            .session_manager
            .force_new_for_channel(channel, &agent_id)
            .await;
    }

    async fn execute_compact(&self, session_id: &str, instruction: Option<String>) {
        if let Some(sh) = self.session_handler.as_ref() {
            let compact_cmd = match &instruction {
                Some(inst) => format!("/compact {}", inst),
                None => "/compact".to_string(),
            };
            sh.handle_compact_command(session_id, &compact_cmd).await;
        }
    }

    async fn execute_system_append(&self, session_id: &str, action: &SystemAppendAction) {
        let cs: Option<Arc<RwLock<ConversationSession>>> = self
            .session_manager
            .get_conversation_session(session_id)
            .await;
        let Some(cs) = cs else {
            if let Some(sh) = self.session_handler.as_ref() {
                sh.send_reply("session 不存在，无法执行系统指令".to_owned())
                    .await;
            }
            return;
        };
        let mut cs = cs.write().await;
        match action {
            SystemAppendAction::Add(text) => {
                cs.add_system_append(text.clone());
            }
            SystemAppendAction::Clear => {
                cs.clear_system_appends();
            }
        }
    }

    async fn execute_set_reasoning(
        &self,
        session_id: &str,
        level: closeclaw_session::persistence::ReasoningLevel,
    ) {
        let cs: Option<Arc<RwLock<ConversationSession>>> = self
            .session_manager
            .get_conversation_session(session_id)
            .await;
        let Some(cs) = cs else {
            if let Some(sh) = self.session_handler.as_ref() {
                sh.send_reply("session 不存在，无法设置推理深度".to_owned())
                    .await;
            }
            return;
        };
        cs.write().await.set_reasoning_level(level);
    }

    async fn execute_set_verbosity(
        &self,
        session_id: &str,
        level: closeclaw_common::VerbosityLevel,
    ) {
        let cs: Option<Arc<RwLock<ConversationSession>>> = self
            .session_manager
            .get_conversation_session(session_id)
            .await;
        let Some(cs) = cs else {
            if let Some(sh) = self.session_handler.as_ref() {
                sh.send_reply("session 不存在，无法设置输出详细度".to_owned())
                    .await;
            }
            return;
        };
        cs.write().await.set_verbosity_level(level);
    }

    async fn execute_exec(
        &self,
        _session_id: &str,
        agent_id: &str,
        command: &str,
    ) -> Vec<ContentBlock> {
        let command = command.trim();
        if command.is_empty() {
            return vec![ContentBlock::Text("用法：/exec <command>".to_owned())];
        }

        let parts: Vec<String> = shlex::split(command).unwrap_or_else(|| vec![command.to_owned()]);
        let cmd = parts.first().cloned().unwrap_or_default();
        let args = parts[1..].to_vec();

        match self.check_command_permission(agent_id, &cmd, &args).await {
            Ok(()) => self.run_command(&cmd, &args).await,
            Err(blocks) => blocks,
        }
    }
}

// ── GatewaySlashExecutor inherent methods ──────────────────────────────

impl GatewaySlashExecutor {
    /// Check permission for a command execution request.
    /// Returns `Ok(())` if allowed, or `Err(blocks)` with a denial message.
    async fn check_command_permission(
        &self,
        agent_id: &str,
        cmd: &str,
        args: &[String],
    ) -> Result<(), Vec<ContentBlock>> {
        let Some(engine) = self.permission_engine.as_ref() else {
            return Err(vec![ContentBlock::Text(
                "无权限：权限引擎未配置".to_owned(),
            )]);
        };
        let caller = Caller {
            user_id: "owner".to_owned(),
            agent: agent_id.to_owned(),
            creator_id: String::new(),
        };
        let request = PermissionRequest::WithCaller {
            caller,
            request: PermissionRequestBody::CommandExec {
                agent: agent_id.to_owned(),
                cmd: cmd.to_owned(),
                args: args.to_vec(),
            },
        };
        let response = engine.evaluate(request, None);
        if let PermissionResponse::Denied { reason, .. } = response {
            return Err(vec![ContentBlock::Text(format!("无权限：{reason}"))]);
        }
        Ok(())
    }

    /// Execute a command and format stdout/stderr into ContentBlocks.
    async fn run_command(&self, cmd: &str, args: &[String]) -> Vec<ContentBlock> {
        let result = tokio::process::Command::new(cmd).args(args).output().await;
        match result {
            Ok(output) => {
                let mut blocks = Vec::new();
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stdout.is_empty() {
                    blocks.push(ContentBlock::Text(stdout.to_string()));
                }
                if !stderr.is_empty() {
                    blocks.push(ContentBlock::Text(format!("[stderr] {stderr}")));
                }
                if blocks.is_empty() {
                    let code = output.status.code().unwrap_or(-1);
                    blocks.push(ContentBlock::Text(format!("命令执行完成，退出码：{code}")));
                }
                blocks
            }
            Err(e) => vec![ContentBlock::Text(format!("命令执行失败：{e}"))],
        }
    }
}
