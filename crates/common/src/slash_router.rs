//! Slash command router trait and related types.
//!
//! Decouples the gateway from the concrete slash command dispatcher,
//! allowing the dispatcher to be swapped or mocked.

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::processor::ContentBlock;

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

/// Action produced by execute_slash_result for the Gateway to dispatch.
#[derive(Debug)]
pub enum ReplyAction {
    /// Send a content-block reply to the user (routed through outbound
    /// Processor Chain: Verbosity filtering → DslParser → outbound logging
    /// → IM Adapter rendering).
    Reply(Vec<ContentBlock>),
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
/// Executor trait for slash command side effects.
///
/// Implemented by the Gateway, which has access to the full
/// `SessionManager` and `SessionMessageHandler`. This trait breaks
/// the circular dependency: common defines the interface, gateway
/// provides the implementation.
#[async_trait]
pub trait SlashEffectExecutor: Send + Sync {
    /// Stop the current LLM turn for the session.
    async fn execute_stop(&self, session_id: &str);

    /// Create a new session for the given channel.
    async fn execute_new_session(&self, session_id: &str, channel: &str);

    /// Trigger context compaction with an optional custom instruction.
    async fn execute_compact(&self, session_id: &str, instruction: Option<String>);

    /// Apply a system prompt append/clear action.
    async fn execute_system_append(&self, session_id: &str, action: &SystemAppendAction);

    /// Set the reasoning level for the session.
    async fn execute_set_reasoning(
        &self,
        session_id: &str,
        level: crate::session_types::ReasoningLevel,
    );

    /// Set the verbosity level for the session.
    async fn execute_set_verbosity(
        &self,
        session_id: &str,
        level: crate::verbosity::VerbosityLevel,
    );

    /// Execute a shell command for the given agent.
    ///
    /// The implementation evaluates command-level permissions via the
    /// permission engine, then runs the command and returns output as
    /// `ContentBlock::Text`. Returns a rejection message on permission denial.
    async fn execute_exec(
        &self,
        session_id: &str,
        agent_id: &str,
        command: &str,
    ) -> Vec<ContentBlock>;
}

/// Context for slash command side-effect dispatch.
///
/// Carries session/channel identity, a reply channel, and an executor
/// for the Gateway to dispatch side effects.
pub struct SideEffectContext {
    /// Session ID where the slash command was invoked.
    pub session_id: String,
    /// Channel identifier (e.g. "feishu", "telegram").
    pub channel: String,
    /// Session manager for state queries.
    pub session_manager: Arc<dyn crate::SessionLookup>,
    /// Sender for [`ReplyAction`]s.
    pub reply_tx: mpsc::Sender<ReplyAction>,
    /// Executor for slash command side effects.
    pub executor: Arc<dyn SlashEffectExecutor>,
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

    /// Get a handler by command name.
    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>>;
}

/// Trait for slash command handlers.
///
/// Implementors define which commands they handle, a description for help
/// text, whether the command is immediate (responds even when LLM is busy),
/// and the async execution logic.
#[async_trait]
pub trait SlashHandler: Send + Sync {
    /// Command names (without the leading `/`).
    fn commands(&self) -> &[&str];

    /// Short description (for /help listing).
    fn description(&self) -> &str;

    /// Whether this is an immediate command (responds even when LLM is busy).
    fn immediate(&self, _cmd: &str) -> bool {
        false
    }

    /// Whether this command requires permission evaluation before execution.
    fn requires_permission(&self) -> bool {
        false
    }

    /// Execute the command with the given arguments and context.
    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult;
}

/// Trait for dispatching slash commands to handlers.
///
/// Provides handler lookup and command metadata.
#[async_trait]
pub trait SlashDispatcherTrait: Send + Sync {
    /// Get a handler by command name.
    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>>;

    /// Check whether a command is immediate.
    fn is_immediate(&self, command: &str) -> bool;
}
