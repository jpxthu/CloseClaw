//! Abstract tool-process kill adapter and session registration trait.
//!
//! [`KillHandle`] is the cross-crate abstraction for cancelling
//! in-flight tool processes. It lives in `common` so both `llm`
//! (which owns `ConversationSession`) and `tools` (which owns the
//! concrete adapters like `BashKillHandle`) can reference it without
//! a circular dependency.
//!
//! [`ToolSession`] provides a minimal registration surface so the
//! Tool trait can live in `common` without depending on
//! `ConversationSession` directly.

use std::io;
use std::sync::Arc;

use async_trait::async_trait;

// ---------------------------------------------------------------------------
// KillHandle — abstract process kill adapter
// ---------------------------------------------------------------------------

/// Abstract tool-process kill operation.
///
/// Implemented by adapter types in `closeclaw-tools` (foreground
/// child processes, background tasks) and by test doubles.
///
/// `kill()` must be safe to call multiple times — callers invoke
/// every registered handle exactly once per stop call, and adapters
/// must be idempotent (e.g. foreground `BashKillHandle` uses
/// `start_kill()`, which is a no-op after the child has already been
/// reaped).
pub trait KillHandle: Send + Sync {
    /// Request termination of the underlying process / task.
    ///
    /// Returns `Ok(())` on success (idempotent re-`kill` is also
    /// success). The caller does not wait for the process to actually
    /// exit — the stop path enforces a wall-clock budget via
    /// `tokio::time::timeout`.
    fn kill(&self) -> io::Result<()>;
}

// ---------------------------------------------------------------------------
// ToolSession — registration surface for tool kill handles
// ---------------------------------------------------------------------------

/// Minimal session interface for tool-handle registration.
///
/// This trait lives in `common` so that `ToolContext` can reference a
/// session without depending on `ConversationSession` (which lives in
/// the `llm` crate). The concrete implementation wraps
/// `ConversationSession::register_tool_handle`.
#[async_trait]
pub trait ToolSession: Send + Sync {
    /// Register a kill handle for a given tool call.
    ///
    /// The session retains the handle until the call completes or is
    /// cancelled.
    async fn register_tool_handle(&self, call_id: String, handle: Arc<dyn KillHandle>);

    /// Register a tool call for pending-operation tracking.
    ///
    /// Called before a tool forks (spawns a subprocess or background
    /// task). The session records the tool name and args summary so
    /// that [`persist_pending_checkpoint`](Self::persist_pending_checkpoint)
    /// can include it in the next checkpoint.
    async fn register_tool_call(
        &self,
        _call_id: String,
        _tool_name: String,
        _args_summary: String,
    ) {
    }

    /// Deregister a tool call after it completes.
    ///
    /// Called after the tool result is available. The session removes
    /// the tool from its pending-operation set.
    ///
    /// Note: terminal `update_tool_state` calls (Completed, Failed,
    /// Terminated, TimedOut) now auto-remove the entry from the tracking
    /// map. This method is retained as an idempotent cleanup for edge
    /// cases where the entry may still exist.
    async fn deregister_tool_call(&self, _call_id: String) {}

    /// Updates the state of a registered tool call.
    ///
    /// Called to transition a tool through its lifecycle states
    /// (e.g. `Pending → RunningForeground → Completed`).
    async fn update_tool_state(&self, _call_id: &str, _state: crate::ToolExecState) {}

    /// Register a child session for pending-operation tracking.
    ///
    /// Called before a child session starts processing. The session records
    /// the agent_id and task summary so that
    /// [`persist_pending_checkpoint`](Self::persist_pending_checkpoint)
    /// can include it in the next checkpoint.
    async fn register_child_state(
        &self,
        _child_id: String,
        _agent_id: String,
        _task_summary: String,
    ) {
    }

    /// Deregister a child session after it completes.
    ///
    /// Called when the child session finishes or is terminated. The session
    /// removes the child from its pending-operation set.
    async fn deregister_child_state(&self, _child_id: String) {}

    /// Persist a checkpoint with the current pending operations.
    ///
    /// Async fire-and-forget: failures are logged at warn level and
    /// must not block the caller. Called after `register_tool_call`
    /// and `deregister_tool_call` so that crash recovery can detect
    /// in-flight operations.
    async fn persist_pending_checkpoint(&self) {}

    /// Returns a reference to the manual backgrounding notify signal.
    ///
    /// Tools can await `signal.notified()` inside `tokio::select!`
    /// to react to user-initiated manual backgrounding requests.
    /// Returns `None` if the session does not support manual
    /// backgrounding (e.g. test doubles).
    fn manual_background_notify(&self) -> Option<Arc<tokio::sync::Notify>> {
        None
    }

    /// Enter active Waiting state (yielding).
    ///
    /// Called by `sessions_yield` tool to signal that the session
    /// should enter Waiting state. The Gateway detects this state
    /// after the LLM call completes and skips draining pending messages.
    fn enter_waiting(&self) {}

    /// Exit active Waiting state and resume normal processing.
    fn exit_waiting(&self) {}

    /// Returns `true` if the session is in active Waiting (yielding).
    fn is_waiting(&self) -> bool {
        false
    }
}
