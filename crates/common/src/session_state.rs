//! Three-dimensional execution state for `ConversationSession`.
//!
//! See `docs/design/session/session-execution.md` for the full state
//! model and transition rules.

use serde::{Deserialize, Serialize};

/// State of the LLM interaction for this session.
///
/// Transitions:
/// - `Idle` → `Requesting` when an LLM request is dispatched
/// - `Requesting` → `Receiving` on first streaming token
/// - `Requesting` → `Idle` when a non-streaming response completes
/// - `Receiving` → `Idle` when the stream ends
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Requesting/Receiving are set by gateway in a later step.
pub enum LlmState {
    /// No LLM interaction in progress.
    #[default]
    Idle,
    /// LLM request dispatched, awaiting response.
    Requesting,
    /// Streaming response in progress (first token received).
    Receiving,
}

/// State of a single tool call tracked by this session.
///
/// `RunningForeground` blocks the session (no new LLM request accepted).
/// `RunningBackground` does not block; the process handle is retained
/// so the result can be injected back into the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Some variants are only constructed by future tool lifecycle integration.
pub enum ToolExecState {
    /// Tool call registered but not yet started.
    Pending,
    /// Executing in foreground; session is blocked on this tool.
    RunningForeground,
    /// Executing in background; session may continue.
    RunningBackground,
    /// Tool finished successfully.
    Completed,
    /// Tool failed with an error.
    Failed,
    /// Tool was explicitly terminated (e.g. by `/stop`).
    Terminated,
    /// Tool exceeded its time budget.
    TimedOut,
}

/// State of a single child session tracked by this session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Some variants are only constructed by future child lifecycle integration.
pub enum ChildSessionState {
    /// Child session is still running.
    Running,
    /// Child session completed successfully.
    Completed,
    /// Child session was explicitly terminated.
    Terminated,
    /// Child session errored.
    Errored,
}

/// Completion status of a child session, used in [`AnnounceEvent`]
/// to convey the final outcome to the parent session.
///
/// This is a snapshot of [`ChildSessionState`] taken at announce time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChildCompletionStatus {
    /// Child session completed its task successfully.
    Completed,
    /// Child session finished with an error.
    Errored,
    /// Child session was explicitly terminated (e.g. via forceful kill).
    Terminated,
}

/// Overall session execution status derived from the three dimensions
/// (LLM, tool, child session). See `docs/design/session/session-execution.md`
/// for the full state table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionExecStatus {
    /// Fully idle: no LLM, no tool, no child session activity.
    Idle,
    /// Idle on the LLM/foreground axis, but background tools are running.
    /// The session can still accept new input.
    IdleWithBackgroundTasks,
    /// Waiting on a running child session to complete.
    Waiting,
    /// LLM interaction or foreground tool execution in progress.
    Busy,
}
