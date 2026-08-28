//! Four-dimensional execution state for `ConversationSession`.
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

impl ToolExecState {
    /// Returns `true` when the state is terminal (Completed, Failed,
    /// Terminated, or TimedOut). Terminal-state tools should be
    /// deregistered immediately from the tracking map.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ToolExecState::Completed
                | ToolExecState::Failed
                | ToolExecState::Terminated
                | ToolExecState::TimedOut
        )
    }
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

/// Four-dimensional activity snapshot of a session.
///
/// Each boolean represents an independent activity dimension:
/// - `llm_active`: LLM request or streaming response in progress
///   (`LlmState::Requesting | Receiving`)
/// - `foreground_tool_active`: A tool call is pending or executing in
///   the foreground (`ToolExecState::Pending | RunningForeground`)
/// - `background_tool_active`: A tool call is executing in the
///   background (`ToolExecState::RunningBackground`)
/// - `child_active`: A child session is still running
///   (`ChildSessionState::Running`)
///
/// See `docs/design/session/session-execution.md` for the state model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SessionActivityDimensions {
    /// LLM request or streaming response is in progress.
    pub llm_active: bool,
    /// A tool call is pending or executing in the foreground.
    pub foreground_tool_active: bool,
    /// A tool call is executing in the background.
    pub background_tool_active: bool,
    /// A child session is running.
    pub child_active: bool,
}

impl SessionActivityDimensions {
    /// Returns `true` when **any** dimension is active.
    pub fn any_active(&self) -> bool {
        self.llm_active
            || self.foreground_tool_active
            || self.background_tool_active
            || self.child_active
    }
}

/// Overall session execution status derived from the four dimensions
/// (LLM, foreground tool, background tool, child session). See
/// `docs/design/session/session-execution.md` for the full state table.
///
/// Only two output states: `Idle` and `Busy`. `Waiting` is a special
/// case when the session is actively yielding (`is_yielding=true`).
/// `background_tool_active` and `child_active` do NOT affect the
/// idle/busy determination — they are exposed via
/// [`SessionActivityDimensions`] for consumers that need them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionExecStatus {
    /// Fully idle: no LLM, no foreground tool activity.
    /// Background tools or children may still be running.
    Idle,
    /// Waiting on a running child session to complete.
    Waiting,
    /// LLM interaction or foreground tool execution in progress.
    Busy,
}
