//! Slash command permission control for the Gateway.
//!
//! Provides `set_slash_dispatcher()`, `set_permission_engine()`, and
//! `dispatch_slash()` for routing slash commands through the permission
//! engine before execution.

use std::sync::Arc;

use crate::llm::session::ChatSession;
use crate::permission::engine::engine_eval::PermissionEngine;
use crate::permission::engine::engine_types::{
    Caller, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
use crate::slash::handler::SlashHandler;
use crate::slash::handler::SystemAppendAction;
use crate::slash::{parse_slash, SlashContext, SlashDispatcher, SlashResult};

use super::{Gateway, HandleResult};

impl Gateway {
    /// Install the slash command dispatcher.
    pub async fn set_slash_dispatcher(&self, dispatcher: Arc<SlashDispatcher>) {
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
    /// 2. `handler.requires_permission() == true` → 调用 `permission_engine.evaluate()`；
    ///    返回 `Denied` 时回复"无权限"并返回 `SlashHandled`
    /// 3. `handler.requires_permission() == false` → 直接分派 handler
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

        if !self
            .check_slash_permission(cmd, sender_id, session_id)
            .await
        {
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

        if let PermissionResponse::Denied { reason, .. } = engine.evaluate(request, None) {
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

    /// Invoke the handler with a constructed `SlashContext`, then route the
    /// returned `SlashResult` to the appropriate side effect.
    async fn execute_and_route(
        &self,
        handler: &dyn SlashHandler,
        cmd_name: &str,
        args: &str,
        session_id: &str,
        sender_id: Option<&str>,
        channel: &str,
    ) -> Option<HandleResult> {
        let ctx = SlashContext {
            command: cmd_name.to_owned(),
            sender_id: sender_id.unwrap_or("").to_owned(),
            session_id: session_id.to_owned(),
            channel: channel.to_owned(),
        };
        let result = handler.handle(args, &ctx).await;

        match result {
            SlashResult::Reply(text) => {
                if let Some(sh) = self.session_handler.as_ref() {
                    sh.send_reply(text).await;
                }
            }
            SlashResult::Compact { instruction } => {
                if let Some(sh) = self.session_handler.as_ref() {
                    let compact_cmd = match &instruction {
                        Some(inst) => format!("/compact {}", inst),
                        None => "/compact".to_string(),
                    };
                    sh.handle_compact_command(session_id, &compact_cmd).await;
                }
            }
            SlashResult::Exec { command: _ } => {
                // Step 1.2 占位：ExecHandler 经权限引擎放行后，先以 Reply 模式
                // 通知用户命令已提交审批。后续步骤接完整审批流。
                if let Some(sh) = self.session_handler.as_ref() {
                    sh.send_reply(format!("命令已提交审批：/{cmd_name}")).await;
                }
            }
            SlashResult::SetReasoning { level } => {
                if let Some(cs) = self
                    .session_manager
                    .get_conversation_session(session_id)
                    .await
                {
                    cs.write().await.set_reasoning_level(level.clone());
                    if let Some(sh) = self.session_handler.as_ref() {
                        sh.send_reply(format!("推理深度已设置为 {:?}", level)).await;
                    }
                } else {
                    if let Some(sh) = self.session_handler.as_ref() {
                        sh.send_reply("当前会话未激活，无法设置推理深度".to_owned())
                            .await;
                    }
                }
            }
            SlashResult::SystemAppend {
                action: SystemAppendAction::Add(content),
            } => {
                if let Some(cs) = self
                    .session_manager
                    .get_conversation_session(session_id)
                    .await
                {
                    let mut session = cs.write().await;
                    let index = session.add_system_append(content);
                    if let Some(sh) = self.session_handler.as_ref() {
                        sh.send_reply(format!("已追加指令（序号 {index}）")).await;
                    }
                } else {
                    if let Some(sh) = self.session_handler.as_ref() {
                        sh.send_reply("当前会话未激活，无法追加指令".to_owned())
                            .await;
                    }
                }
            }
            SlashResult::SystemAppend {
                action: SystemAppendAction::Clear,
            } => {
                if let Some(cs) = self
                    .session_manager
                    .get_conversation_session(session_id)
                    .await
                {
                    let mut session = cs.write().await;
                    let count = session.clear_system_appends();
                    if let Some(sh) = self.session_handler.as_ref() {
                        sh.send_reply(format!("已清除 {count} 条追加指令")).await;
                    }
                } else {
                    if let Some(sh) = self.session_handler.as_ref() {
                        sh.send_reply("当前会话未激活，无法清除指令".to_owned())
                            .await;
                    }
                }
            }
            SlashResult::NewSession => {
                let agent_id = self
                    .session_manager
                    .get_chat_id(session_id)
                    .await
                    .unwrap_or_default();
                let new_session_id = self
                    .session_manager
                    .force_new_for_channel(channel, &agent_id)
                    .await;
                if let Some(sh) = self.session_handler.as_ref() {
                    sh.send_reply(format!("已创建新 session：{new_session_id}"))
                        .await;
                }
            }
            SlashResult::Stop => {
                match self
                    .session_manager
                    .get_conversation_session(session_id)
                    .await
                {
                    None => {
                        if let Some(sh) = self.session_handler.as_ref() {
                            sh.send_reply("当前会话未激活".to_owned()).await;
                        }
                    }
                    Some(conv) => {
                        let busy = {
                            let cs = conv.read().await;
                            cs.is_llm_busy()
                        };
                        if busy {
                            let mut cs = conv.write().await;
                            cs.cancel_token.cancel();
                            // Cascade stop to child handles.
                            {
                                let child_handles = cs
                                    .child_handles
                                    .read()
                                    .expect("child_handles lock poisoned");
                                let handles_to_stop: Vec<_> =
                                    child_handles.values().filter_map(|w| w.upgrade()).collect();
                                drop(child_handles);
                                for child in handles_to_stop {
                                    let child_cs = child.read().await;
                                    child_cs.cancel_token.cancel();
                                }
                            }
                            cs.clear_pending();
                        }
                        if let Some(sh) = self.session_handler.as_ref() {
                            sh.send_reply("已停止当前任务".to_owned()).await;
                        }
                    }
                }
            }
            SlashResult::SetMode(_) => {
                tracing::warn!(
                    cmd = cmd_name,
                    "SlashResult::SetMode not yet routed through dispatch_slash"
                );
            }
            SlashResult::Unknown(_) => {
                // Should not happen — dispatcher only invokes handler on match.
                tracing::debug!("SlashResult::Unknown returned from handler");
            }
        }

        Some(HandleResult::SlashHandled)
    }
}
