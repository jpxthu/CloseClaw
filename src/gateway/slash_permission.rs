//! Slash command permission control for the Gateway.
//!
//! Provides `set_slash_dispatcher()`, `set_permission_engine()`, and
//! `dispatch_slash()` for routing slash commands through the permission
//! engine before execution.

use std::sync::Arc;

use crate::permission::engine::engine_eval::PermissionEngine;
use crate::permission::engine::engine_types::{
    Caller, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
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

        let (cmd, args) = parse_slash(content)?;
        let handler = dispatcher.get_handler(cmd)?;

        // Owner bypasses all permission checks.
        let is_owner = sender_id == Some("owner");

        if !is_owner && handler.requires_permission() {
            let engine_guard = self.permission_engine.read().await;
            if let Some(engine) = engine_guard.as_ref() {
                let request = PermissionRequest::WithCaller {
                    caller: Caller {
                        user_id: sender_id.unwrap_or("").to_owned(),
                        agent: "gateway".into(),
                        creator_id: String::new(),
                    },
                    request: PermissionRequestBody::SlashCommand {
                        agent: "gateway".into(),
                        command: cmd.to_owned(),
                    },
                };
                match engine.evaluate(request, None) {
                    PermissionResponse::Denied { .. } => {
                        // TODO: send "permission denied" reply via adapter
                        return Some(HandleResult::SlashHandled);
                    }
                    PermissionResponse::Allowed { .. } => {}
                }
            }
        }

        // Execute the handler.
        let ctx = SlashContext {
            sender_id: sender_id.unwrap_or("").to_owned(),
            session_id: session_id.to_owned(),
        };
        let _result = handler.handle(args, &ctx).await;
        // TODO: route SlashResult::Reply(msg) back through the adapter

        Some(HandleResult::SlashHandled)
    }
}
