use crate::context::SlashContext;
use closeclaw_common::slash_router::SlashResult;

/// Trait for slash command handlers.
///
/// Implementors define which commands they handle, a description for help
/// text, whether the command is immediate (responds even when LLM is busy),
/// and the async execution logic.
#[async_trait::async_trait]
pub trait SlashHandler: Send + Sync {
    /// Command names (without the leading `/`).
    fn commands(&self) -> &[&str];

    /// Short description (for /help listing).
    fn description(&self) -> &str;

    /// Whether this is an immediate command (responds even when LLM is busy).
    fn immediate(&self, _cmd: &str) -> bool {
        false
    }

    /// Whether this command requires permission evaluation by the permission
    /// engine before execution. High-risk handlers (e.g. `/exec`) override this
    /// to return `true`; the default is `false` (safe handlers execute directly).
    fn requires_permission(&self) -> bool {
        false
    }

    /// Execute the command with the given arguments and context.
    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult;
}
