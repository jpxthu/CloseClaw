//! Slash command router trait and related types.
//!
//! Decouples the gateway from the concrete slash command dispatcher,
//! allowing the dispatcher to be swapped or mocked.

use async_trait::async_trait;

/// Execution context for a slash command invocation.
#[derive(Debug, Clone)]
pub struct SlashContext {
    /// The slash command name (without the leading `/`).
    ///
    /// For multi-command handlers (e.g. `WorkdirHandler` handling `cd`,
    /// `pwd`, `git`) this lets `handle()` branch on the invoked subcommand.
    pub command: String,
    /// Open ID of the message sender.
    pub sender_id: String,
    /// Session ID where the command was invoked.
    pub session_id: String,
    /// Channel identifier (e.g. "feishu", "telegram").
    pub channel: String,
}

/// Result of a slash command dispatch.
#[derive(Debug)]
pub enum SlashResult {
    /// Reply with a text message.
    Reply(String),
    /// Switch mode (future).
    SetMode(String),
    /// Create a new session.
    NewSession,
    /// Stop the current run.
    Stop,
    /// Compact context with optional instruction.
    Compact { instruction: Option<String> },
    /// Append to system prompt.
    SystemAppend { action: SystemAppendAction },
    /// Execute a sub-command.
    Exec { command: String },
    /// Set the reasoning level for the current session.
    SetReasoning {
        level: crate::session_types::ReasoningLevel,
    },
    /// Set the verbosity level for the current session.
    SetVerbosity {
        level: crate::verbosity::VerbosityLevel,
    },
    /// Unknown command — no handler matched.
    Unknown(String),
}

/// Action for the `SystemAppend` slash result.
#[derive(Debug, Clone)]
pub enum SystemAppendAction {
    /// Append a new system prompt instruction.
    Add(String),
    /// Clear all appended system prompt instructions.
    Clear,
}

/// Action produced by `SlashResult::execute` for the Gateway to dispatch.
#[derive(Debug)]
pub enum ReplyAction {
    /// Send a text reply to the user.
    Reply(String),
    /// Trigger manual compaction.
    TriggerCompact { instruction: Option<String> },
    /// No action needed.
    Nothing,
}

/// Trait for routing slash commands to handlers.
///
/// Implemented by `SlashDispatcher` in the slash crate; used by the
/// gateway to dispatch commands without a direct dependency on the
/// slash module.
#[async_trait]
pub trait SlashRouter: Send + Sync {
    /// Dispatch a slash command and return the result.
    ///
    /// # Arguments
    /// * `content` — raw message content starting with `/`
    /// * `ctx` — execution context (sender, session, channel)
    ///
    /// Returns `Some(SlashResult)` if the command was recognized,
    /// or `None` if the content is not a slash command.
    async fn dispatch(&self, content: &str, ctx: &SlashContext) -> Option<SlashResult>;

    /// Check whether a command is immediate (responds even when LLM is busy).
    fn is_immediate(&self, command: &str) -> bool;

    /// Parse a slash command from raw content.
    ///
    /// Returns `Some((command, args))` where `command` is without the
    /// leading `/` and `args` is the remainder. Returns `None` if the
    /// content does not start with `/`.
    fn parse(content: &str) -> Option<(&str, &str)> {
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
}
