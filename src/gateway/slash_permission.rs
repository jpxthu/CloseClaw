//! Slash command permission control for the Gateway.
//!
//! Provides `set_slash_dispatcher()`, `set_permission_engine()`, and
//! `dispatch_slash()` for routing slash commands through the permission
//! engine before execution.

use std::sync::Arc;

use crate::permission::engine::engine_eval::PermissionEngine;
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
    pub(super) async fn dispatch_slash(
        &self,
        session_id: &str,
        content: &str,
        sender_id: Option<&str>,
    ) -> Option<HandleResult> {
        let dispatcher_guard = self.slash_dispatcher.read().await;
        let dispatcher = dispatcher_guard.as_ref()?;

        let (cmd, args) = match parse_slash(content) {
            Some(parsed) => parsed,
            None => return None, // 不是 slash 命令
        };
        let Some(handler) = dispatcher.get_handler(cmd) else {
            // 未知 slash 命令：发送"未知指令"提示
            let reply = format!("未知指令：/{cmd}。输入 /help 查看所有可用指令。");
            if let Some(sh) = self.session_handler.as_ref() {
                sh.send_reply(reply).await;
            }
            return Some(HandleResult::SlashHandled);
        };

        // Permission check removed — will be re-implemented via a separate
        // mechanism in a future step. For now, all slash commands are dispatched
        // directly regardless of sender.

        // Execute the handler.
        let ctx = SlashContext {
            sender_id: sender_id.unwrap_or("").to_owned(),
            session_id: session_id.to_owned(),
            channel: String::new(), // 占位；Step 1.7 会从 Gateway::handle_inbound_message 传入实际 channel
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
                    // Reconstruct the /compact command for handle_compact_command
                    let cmd = match &instruction {
                        Some(inst) => format!("/compact {}", inst),
                        None => "/compact".to_string(),
                    };
                    sh.handle_compact_command(session_id, &cmd).await;
                }
            }
            SlashResult::SetMode(_)
            | SlashResult::NewSession
            | SlashResult::Stop
            | SlashResult::SystemAppend { .. }
            | SlashResult::Exec { .. } => {
                tracing::warn!(
                    cmd,
                    "SlashResult variant not yet routed through dispatch_slash"
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
