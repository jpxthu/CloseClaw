use std::sync::Arc;

use crate::context::SlashContext;
use crate::debug_log::{emit_slash_event, SlashDebugLogContext, SlashEmitEventParams};
use crate::handler::SlashHandler;
use crate::registry::HandlerRegistry;
use closeclaw_common::slash_router::SlashResult;
use closeclaw_common::slash_router::SlashRouter;
use closeclaw_debug_log::{DebugLog, LogLevel};

/// Parses a slash command from raw message content.
///
/// Returns `Some((command, args))` where `command` is the name without the
/// leading `/` and `args` is the remainder of the string (possibly empty).
/// Returns `None` if the content does not start with `/`.
pub fn parse_slash(content: &str) -> Option<(&str, &str)> {
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

/// Top-level dispatcher that routes slash commands to registered handlers.
pub struct SlashDispatcher {
    registry: Arc<HandlerRegistry>,
}

/// Build a [`SlashContext`] with the command field populated.
///
/// Both the inherent `dispatch` and trait `dispatch` need the same
/// construction; this helper keeps them in sync.
fn build_ctx(cmd: &str, ctx: &SlashContext) -> SlashContext {
    SlashContext {
        command: cmd.to_owned(),
        sender_id: ctx.sender_id.clone(),
        session_id: ctx.session_id.clone(),
        channel: ctx.channel.clone(),
    }
}

impl SlashDispatcher {
    /// Create a new dispatcher backed by the given registry.
    pub fn new(registry: HandlerRegistry) -> Self {
        Self {
            registry: Arc::new(registry),
        }
    }

    /// Create a new dispatcher backed by a shared registry (Arc variant).
    pub fn from_shared(registry: Arc<HandlerRegistry>) -> Self {
        Self { registry }
    }

    /// Look up a handler by command name (without the leading `/`).
    pub fn get_handler(&self, command: &str) -> Option<Arc<dyn SlashHandler>> {
        self.registry.get_arc(command)
    }

    /// Dispatch a raw message content string.
    ///
    /// If the content is a recognized slash command, the corresponding handler
    /// is invoked. Otherwise returns [`SlashResult::Unknown`].
    ///
    /// **Note**: Unlike the [`SlashRouter::dispatch`] trait method which returns
    /// `None` for non-`/` content, this inherent method returns
    /// [`SlashResult::Unknown`] for all unrecognised content (including plain
    /// text). The inherent method is retained for backward compatibility with
    /// daemon/cli call sites that pre-date the trait.
    pub async fn dispatch(&self, content: &str, ctx: &SlashContext) -> SlashResult {
        let Some((cmd, args)) = parse_slash(content) else {
            return SlashResult::Unknown(content.to_owned());
        };
        let Some(handler) = self.registry.get_arc(cmd) else {
            return SlashResult::Unknown(content.to_owned());
        };
        handler.handle(args, &build_ctx(cmd, ctx)).await
    }

    /// Dispatch with debug-log emission.
    ///
    /// Same as [`dispatch`](Self::dispatch) but emits structured debug-log
    /// events at command detection and dispatch nodes.
    pub async fn dispatch_with_debug_log(
        &self,
        content: &str,
        ctx: &SlashContext,
        debug_log: Option<&DebugLog>,
        trace_id: &str,
        session_key: Option<&str>,
    ) -> SlashResult {
        let Some((cmd, args)) = parse_slash(content) else {
            return SlashResult::Unknown(content.to_owned());
        };

        // slash.command (中间状态): command detected
        emit_slash_event(SlashEmitEventParams {
            ctx: SlashDebugLogContext::new(debug_log, trace_id, session_key),
            level: LogLevel::Info,
            source_module: "slash",
            event_type: "slash.command",
            payload: serde_json::json!({
                "command": cmd,
                "args": args,
            }),
            parent: None,
        });

        let Some(handler) = self.registry.get_arc(cmd) else {
            return SlashResult::Unknown(content.to_owned());
        };
        let result = handler.handle(args, &build_ctx(cmd, ctx)).await;

        // slash.dispatch (关键事件): dispatch completed
        let result_type = match &result {
            SlashResult::Reply(_) => "reply",
            SlashResult::Unknown(_) => "unknown",
            SlashResult::SetMode { .. } => "set_mode",
            SlashResult::NewSession => "new_session",
            SlashResult::Stop { .. } => "stop",
            SlashResult::Compact { .. } => "compact",
            SlashResult::SystemAppend { .. } => "system_append",
            SlashResult::Exec { .. } => "exec",
            SlashResult::SetReasoning { .. } => "set_reasoning",
            SlashResult::SetVerbosity { .. } => "set_verbosity",
        };
        emit_slash_event(SlashEmitEventParams {
            ctx: SlashDebugLogContext::new(debug_log, trace_id, session_key),
            level: LogLevel::Info,
            source_module: "slash",
            event_type: "slash.dispatch",
            payload: serde_json::json!({
                "command": cmd,
                "result_type": result_type,
            }),
            parent: None,
        });

        result
    }

    /// Check whether a command is an Immediate command (responds even when
    /// the LLM is busy). Returns false for unknown commands.
    ///
    /// `content` is the full raw message content (e.g. `"/mode"` or
    /// `"/mode plan"`). Parses the content to extract command name and
    /// arguments, then delegates to [`SlashHandler::immediate`].
    pub fn is_immediate(&self, content: &str) -> bool {
        let Some((cmd, args)) = parse_slash(content) else {
            return false;
        };
        self.registry
            .get_arc(cmd)
            .map(|h| h.immediate(cmd, args))
            .unwrap_or(false)
    }

    /// Collect all registered (command, handler) pairs.
    pub fn all_handlers(&self) -> Vec<(String, Arc<dyn SlashHandler>)> {
        self.registry.iter_arc()
    }
}

#[async_trait::async_trait]
impl SlashRouter for SlashDispatcher {
    /// Dispatch a raw message content string.
    ///
    /// Returns `Some(SlashResult)` if the content is a slash command
    /// (recognized or unknown). Returns `None` if the content does not
    /// start with `/`.
    async fn dispatch(&self, content: &str, ctx: &SlashContext) -> Option<SlashResult> {
        let (cmd, args) = parse_slash(content)?;
        let handler = self.registry.get_arc(cmd);
        let result = match handler {
            Some(h) => h.handle(args, &build_ctx(cmd, ctx)).await,
            None => SlashResult::Unknown(content.to_owned()),
        };
        Some(result)
    }

    fn is_immediate(&self, content: &str) -> bool {
        let Some((cmd, args)) = parse_slash(content) else {
            return false;
        };
        self.registry
            .get_arc(cmd)
            .map(|h| h.immediate(cmd, args))
            .unwrap_or(false)
    }

    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        self.registry.get(command)
    }
}
