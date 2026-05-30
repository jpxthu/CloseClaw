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
    registry: HandlerRegistry,
}

impl SlashDispatcher {
    /// Create a new dispatcher backed by the given registry.
    pub fn new(registry: HandlerRegistry) -> Self {
        Self { registry }
    }

    /// Look up a handler by command name (without leading `/`).
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
        handler.handle(args, ctx).await
    }
}
