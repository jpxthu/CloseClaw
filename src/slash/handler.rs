use crate::session::persistence::ReasoningLevel;
use crate::slash::context::SlashContext;

/// Action for the `SystemAppend` slash result.
#[derive(Debug, Clone)]
pub enum SystemAppendAction {
    /// Append a new system prompt instruction.
    Add(String),
    /// Clear all appended system prompt instructions.
    Clear,
}

/// Result of a slash command dispatch.
pub enum SlashResult {
    /// Reply with a text message.
    Reply(String),
    /// Switch mode (future).
    SetMode(String),
    /// Create a new session (future).
    NewSession,
    /// Stop the current run (future).
    Stop,
    /// Compact context with optional instruction.
    Compact { instruction: Option<String> },
    /// Append to system prompt (future).
    SystemAppend { action: SystemAppendAction },
    /// Execute a sub-command (future).
    Exec { command: String },
    /// Set the reasoning level for the current session.
    SetReasoning { level: ReasoningLevel },
    /// Unknown command — no handler matched.
    Unknown(String),
}

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
    fn immediate(&self) -> bool {
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
