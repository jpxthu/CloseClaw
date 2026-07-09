//! Slash command permission control for the Gateway.
//!
//! Provides `set_slash_dispatcher()`, `set_permission_engine()`, and
//! `dispatch_slash()` for routing slash commands through the permission
//! engine before execution.

use std::sync::Arc;

use crate::slash_executor::{
    ReplyAction, SideEffectContext, SlashEffectExecutor, SlashResultExecutor,
};
use closeclaw_common::processor::ContentBlock;
use closeclaw_common::slash_router::{
    SlashContext, SlashHandler, SlashResult, SlashRouter, SystemAppendAction,
};
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_types::{
    Caller, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
use closeclaw_session::persistence::PendingMessage;

use super::{Gateway, HandleResult, SessionManager, SessionMessageHandler};
use closeclaw_permission::UserRegistry;
use closeclaw_session::llm_session::ConversationSession;
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
    pub async fn set_permission_engine(&self, engine: Arc<tokio::sync::RwLock<PermissionEngine>>) {
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
    pub(crate) async fn dispatch_slash(
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
                agent: agent_id.clone(),
                command: cmd.to_owned(),
            },
        };

        // Chain-aware permission check: dimension-level intersection
        // with the parent agent chain.
        let response = engine
            .read()
            .await
            .evaluate_with_chain(
                request,
                &*self.session_manager,
                session_id,
                &agent_permissions,
            )
            .await;
        match response {
            PermissionResponse::Denied { reason, .. } => {
                self.send_reply_if_available(&format!("无权限：{reason}"))
                    .await;
                return false;
            }
            PermissionResponse::ApprovalRequired {
                operation_desc,
                risk_level,
                ..
            } => {
                let flow_guard = self.approval_flow.read().await;
                if let Some(flow) = flow_guard.as_ref() {
                    let mut flow = flow.lock().await;
                    if flow
                        .submit_denial(
                            &Caller {
                                user_id: sender_id.unwrap_or("owner").to_owned(),
                                agent: agent_id.clone(),
                                creator_id: String::new(),
                            },
                            &PermissionRequestBody::SlashCommand {
                                agent: agent_id,
                                command: cmd.to_owned(),
                            },
                            risk_level,
                            session_id,
                            false,
                        )
                        .is_some()
                    {
                        self.send_reply_if_available(&format!("⏳ 操作需要审批：{operation_desc}"))
                            .await;
                        return false;
                    }
                }
                self.send_reply_if_available(&format!("无权限：{operation_desc}"))
                    .await;
                return false;
            }
            _ => {}
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

        // PermissionOp is intercepted here — handled directly in the daemon,
        // never enters the normal execute() path.
        if let SlashResult::PermissionOp { op } = &result {
            let reply = self.handle_permission_op(op, sender_id).await;
            if let Some(sh) = self.session_handler.as_ref() {
                sh.send_reply(reply).await;
            }
            return Some(HandleResult::SlashHandled);
        }

        // UserApprove/UserReject are intercepted here — they go through
        // the ApprovalFlow for user registration management.
        if let SlashResult::UserApprove {
            request_id,
            initial_permissions,
        } = &result
        {
            let reply = self
                .handle_user_approve(request_id, initial_permissions, sender_id)
                .await;
            if let Some(sh) = self.session_handler.as_ref() {
                sh.send_reply(reply).await;
            }
            return Some(HandleResult::SlashHandled);
        }
        if let SlashResult::UserReject { request_id } = &result {
            let reply = self.handle_user_reject(request_id, sender_id).await;
            if let Some(sh) = self.session_handler.as_ref() {
                sh.send_reply(reply).await;
            }
            return Some(HandleResult::SlashHandled);
        }

        // Permission check AFTER handler returns SlashResult but BEFORE execute.
        // Handler is allowed to run (returns context), but high-risk side effects
        // are blocked if permission is denied.
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
        let af = self.approval_flow.read().await.clone();
        let executor: Arc<dyn SlashEffectExecutor> = Arc::new(GatewaySlashExecutor {
            session_manager: Arc::clone(&self.session_manager),
            session_handler: self.session_handler.clone(),
            permission_engine: perm_engine,
            approval_flow: af,
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

        Some(HandleResult::SlashHandled)
    }

    /// Handle a [`PermissionOp`] — owner-only permission rule management.
    async fn handle_permission_op(
        &self,
        op: &closeclaw_common::PermissionOperation,
        sender_id: Option<&str>,
    ) -> String {
        if sender_id != Some("owner") {
            return "无权限：仅 Owner 可以执行权限管理操作".to_owned();
        }

        // Path traversal validation for file operations.
        if let Some(paths) = Self::op_file_paths(op) {
            for path in paths {
                if Self::is_path_dangerous(path) {
                    return format!("拒绝：路径包含危险模式 '{path}'");
                }
            }
        }

        let (rule, agent_id) = match Self::build_rule_from_op(op) {
            Some(v) => v,
            None => return "错误：无法构建规则".to_owned(),
        };

        let config_dir = match self.get_config_dir().await {
            Some(dir) => dir,
            None => return "错误：config_dir 未配置".to_owned(),
        };
        if let Err(e) = closeclaw_permission::whitelist::append_rule(&config_dir, &agent_id, rule) {
            return format!("错误：写入规则失败 — {e}");
        }

        // Hot-reload the permission engine with the updated ruleset.
        self.hot_reload_engine(&self.permission_engine, &config_dir, &agent_id)
            .await;

        format!("✅ 已执行：{}", op.describe())
    }

    fn op_file_paths(op: &closeclaw_common::PermissionOperation) -> Option<Vec<&String>> {
        match op {
            closeclaw_common::PermissionOperation::AddFileWhitelist { paths, .. }
            | closeclaw_common::PermissionOperation::AddFileDeny { paths, .. } => {
                Some(paths.iter().collect())
            }
            _ => None,
        }
    }

    fn build_rule_from_op(
        op: &closeclaw_common::PermissionOperation,
    ) -> Option<(closeclaw_permission::Rule, String)> {
        let whitelist = matches!(
            op,
            closeclaw_common::PermissionOperation::AddFileWhitelist { .. }
                | closeclaw_common::PermissionOperation::AddCommandWhitelist { .. }
        );
        let (agent, body, name) = match op {
            closeclaw_common::PermissionOperation::AddFileWhitelist { agent, op, paths }
            | closeclaw_common::PermissionOperation::AddFileDeny { agent, op, paths } => {
                let prefix = if whitelist { "allow" } else { "deny" };
                (
                    agent.clone(),
                    PermissionRequestBody::FileOp {
                        agent: agent.clone(),
                        path: paths.join(","),
                        op: op.clone(),
                    },
                    format!("perm-{prefix}-file-{agent}"),
                )
            }
            closeclaw_common::PermissionOperation::AddCommandWhitelist {
                agent,
                command,
                args,
                ..
            }
            | closeclaw_common::PermissionOperation::AddCommandDeny {
                agent,
                command,
                args,
                ..
            } => {
                let prefix = if whitelist { "allow" } else { "deny" };
                (
                    agent.clone(),
                    PermissionRequestBody::CommandExec {
                        agent: agent.clone(),
                        cmd: command.clone(),
                        args: args.clone(),
                    },
                    format!("perm-{prefix}-cmd-{agent}"),
                )
            }
            // CreateUser goes through the ApprovalFlow, not through
            // the whitelist/deny rule path.
            closeclaw_common::PermissionOperation::CreateUser { .. } => {
                return None;
            }
        };
        let caller = Caller {
            user_id: "owner".to_owned(),
            agent: agent.clone(),
            creator_id: String::new(),
        };
        let rule_fn = if whitelist {
            closeclaw_permission::whitelist::build_whitelist_rule
        } else {
            closeclaw_permission::whitelist::build_deny_rule
        };
        rule_fn(&caller, &body, &name).map(|r| (r, agent))
    }

    async fn hot_reload_engine(
        &self,
        permission_engine: &RwLock<Option<Arc<tokio::sync::RwLock<PermissionEngine>>>>,
        config_dir: &std::path::Path,
        agent_id: &str,
    ) {
        let path = config_dir
            .join("agents")
            .join(agent_id)
            .join("permissions.json");
        if let Ok(json) = tokio::fs::read_to_string(&path).await {
            if let Ok(ruleset) = serde_json::from_str::<closeclaw_permission::RuleSet>(&json) {
                if let Some(engine_arc) = permission_engine.read().await.as_ref() {
                    if let Ok(mut engine) = engine_arc.try_write() {
                        engine.reload_rules(ruleset);
                    }
                }
            }
        }
    }

    fn is_path_dangerous(path: &str) -> bool {
        path.contains("../")
            || path.contains("..\\")
            || path.starts_with('/')
            || (path.len() >= 2 && path.as_bytes()[1] == b':')
            || path.contains('\0')
    }

    /// Handle a `UserApprove` result — register the user via ApprovalFlow.
    async fn handle_user_approve(
        &self,
        request_id: &str,
        initial_permissions: &[closeclaw_common::permission_op::InitialPermissionSet],
        sender_id: Option<&str>,
    ) -> String {
        if sender_id != Some("owner") {
            return "无权限：仅 Owner 可以审批用户注册".to_owned();
        }

        let flow_guard = self.approval_flow.read().await;
        let Some(flow_arc) = flow_guard.as_ref() else {
            return "错误：审批流未配置".to_owned();
        };
        let mut flow = flow_arc.lock().await;
        // Set the selected initial permissions on the pending request.
        flow.set_user_creation_permissions(request_id, initial_permissions.to_vec());
        match flow
            .approve_request(
                request_id,
                closeclaw_permission::approval::ApprovalMode::Once,
            )
            .await
        {
            Ok(true) => {
                let perms: Vec<&str> = initial_permissions.iter().map(|p| p.label()).collect();
                format!("✅ 用户注册已批准（权限: [{}]）", perms.join(", "))
            }
            Ok(false) => "用户注册审批失败：用户可能已注册".to_owned(),
            Err(e) => format!("用户注册审批失败：{:?}", e),
        }
    }

    /// Handle a `UserReject` result — reject the user registration via ApprovalFlow.
    async fn handle_user_reject(&self, request_id: &str, sender_id: Option<&str>) -> String {
        if sender_id != Some("owner") {
            return "无权限：仅 Owner 可以拒绝用户注册".to_owned();
        }

        let flow_guard = self.approval_flow.read().await;
        let Some(flow_arc) = flow_guard.as_ref() else {
            return "错误：审批流未配置".to_owned();
        };
        let mut flow = flow_arc.lock().await;
        if flow.deny_request(request_id) {
            "用户注册已拒绝".to_owned()
        } else {
            "拒绝失败：请求不存在或已处理".to_owned()
        }
    }

    /// Check if a sender is a new unregistered user and auto-submit
    /// a user creation request via the ApprovalFlow.
    ///
    /// When a non-owner, unregistered user sends their first message:
    /// 1. Submit a user creation request via `ApprovalFlow::submit_user_creation()`
    /// 2. Notify the user that their request is pending approval
    /// 3. Return `Some(HandleResult::SlashHandled)` to block further processing
    ///
    /// Returns `None` if the sender is owner, already registered, or no
    /// approval flow is configured.
    pub(crate) async fn check_new_user_registration(
        &self,
        sender_id: &str,
        channel: &str,
    ) -> Option<HandleResult> {
        // Owner doesn't need registration.
        if sender_id == "owner" {
            return None;
        }

        // Load user registry from config_dir/users.json.
        let config_dir = self.get_config_dir().await?;
        let registry_path = config_dir.join("users.json");
        let registry: UserRegistry = tokio::fs::read_to_string(&registry_path)
            .await
            .ok()
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default();

        // Already registered → proceed normally.
        if registry.is_registered(sender_id) {
            return None;
        }

        // New user → submit creation request.
        let flow_guard = self.approval_flow.read().await;
        let Some(flow_arc) = flow_guard.as_ref() else {
            tracing::debug!(
                sender_id,
                "no approval flow configured, cannot register new user"
            );
            return None;
        };
        let mut flow = flow_arc.lock().await;
        match flow.submit_user_creation(sender_id, channel, vec![]) {
            Some(request_id) => {
                tracing::info!(
                    sender_id,
                    channel,
                    request_id = %request_id,
                    "new user registration request auto-submitted"
                );
                if let Some(sh) = self.session_handler.as_ref() {
                    sh.send_reply(format!(
                        "👋 您是新用户，已向 Owner 提交注册申请（请求 ID: {}）。请等待审批。",
                        request_id
                    ))
                    .await;
                }
                Some(HandleResult::SlashHandled)
            }
            None => {
                // Duplicate request or other issue.
                tracing::debug!(
                    sender_id,
                    channel,
                    "user creation request already pending or failed"
                );
                if let Some(sh) = self.session_handler.as_ref() {
                    sh.send_reply("⏳ 您的注册请求正在审批中，请等待。".to_owned())
                        .await;
                }
                Some(HandleResult::SlashHandled)
            }
        }
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
    permission_engine: Option<Arc<tokio::sync::RwLock<PermissionEngine>>>,
    approval_flow: Option<Arc<tokio::sync::Mutex<ApprovalFlow>>>,
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
                // Invalidate static layer cache on clear, so the next
                // prompt build regenerates from current state.
                self.session_manager.invalidate_static_cache().await;
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

    async fn execute_set_mode(&self, session_id: &str, mode: &str) {
        let cs: Option<Arc<RwLock<ConversationSession>>> = self
            .session_manager
            .get_conversation_session(session_id)
            .await;
        let Some(cs) = cs else {
            if let Some(sh) = self.session_handler.as_ref() {
                sh.send_reply("session 不存在，无法设置 mode".to_owned())
                    .await;
            }
            return;
        };
        match closeclaw_common::SessionMode::from_str_opt(mode) {
            Some(parsed) => {
                cs.write().await.set_session_mode(parsed);
            }
            None => {
                tracing::warn!(
                    session_id,
                    mode,
                    "unknown session mode; keeping current mode"
                );
            }
        }
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
        let response = engine.read().await.evaluate(request, None);
        match response {
            PermissionResponse::Denied { reason, .. } => {
                return Err(vec![ContentBlock::Text(format!("无权限：{reason}"))]);
            }
            PermissionResponse::ApprovalRequired {
                operation_desc,
                risk_level,
                ..
            } => {
                if let Some(ref flow) = self.approval_flow {
                    let mut flow = flow.lock().await;
                    if flow
                        .submit_denial(
                            &Caller {
                                user_id: "owner".to_owned(),
                                agent: agent_id.to_owned(),
                                creator_id: String::new(),
                            },
                            &PermissionRequestBody::CommandExec {
                                agent: agent_id.to_owned(),
                                cmd: cmd.to_owned(),
                                args: args.to_vec(),
                            },
                            risk_level,
                            "",
                            false,
                        )
                        .is_some()
                    {
                        return Err(vec![ContentBlock::Text(format!(
                            "⏳ 操作需要审批：{operation_desc}"
                        ))]);
                    }
                }
                return Err(vec![ContentBlock::Text(format!(
                    "无权限：{operation_desc}"
                ))]);
            }
            _ => {}
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
