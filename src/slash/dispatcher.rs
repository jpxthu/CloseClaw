use std::sync::Arc;

use crate::slash::context::SlashContext;
use crate::slash::handler::SlashHandler;
use crate::slash::handler::SlashResult;
use crate::slash::registry::HandlerRegistry;

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
        self.registry.get(command)
    }

    /// Dispatch a raw message content string.
    ///
    /// If the content is a recognized slash command, the corresponding handler
    /// is invoked. Otherwise returns [`SlashResult::Unknown`].
    pub async fn dispatch(&self, content: &str, ctx: &SlashContext) -> SlashResult {
        let Some((cmd, args)) = parse_slash(content) else {
            return SlashResult::Unknown(content.to_owned());
        };
        let Some(handler) = self.registry.get(cmd) else {
            return SlashResult::Unknown(content.to_owned());
        };
        let ctx_with_cmd = SlashContext {
            command: cmd.to_owned(),
            sender_id: ctx.sender_id.clone(),
            session_id: ctx.session_id.clone(),
            channel: ctx.channel.clone(),
        };
        handler.handle(args, &ctx_with_cmd).await
    }

    /// Check whether a command is an Immediate command (responds even when
    /// the LLM is busy). Returns false for unknown commands.
    pub fn is_immediate(&self, command: &str) -> bool {
        self.registry
            .get(command)
            .map(|h| h.immediate(command))
            .unwrap_or(false)
    }

    /// Collect all registered (command, handler) pairs.
    pub fn all_handlers(&self) -> Vec<(String, Arc<dyn SlashHandler>)> {
        self.registry
            .iter()
            .into_iter()
            .map(|(cmd, h)| (cmd, Arc::clone(&h)))
            .collect()
    }
}
