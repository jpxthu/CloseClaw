use crate::slash::context::SlashContext;

/// Result of a slash command dispatch.
pub enum SlashResult {
    /// Reply with a text message.
    Reply(String),
    /// Unknown command — no handler matched.
    Unknown(String),
}

/// Trait for slash command handlers.
///
/// Implementors define a command name, whether elevated permissions are
/// required, and the async execution logic.
#[async_trait::async_trait]
pub trait SlashHandler: Send + Sync {
    /// Command name (without the leading `/`).
    fn name(&self) -> &str;

    /// Whether this command requires elevated permission for non-owners.
    fn requires_permission(&self) -> bool {
        false
    }

    /// Execute the command with the given arguments and context.
    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult;
}
