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
use std::time::{Duration, SystemTime};

use async_trait::async_trait;

// ---------------------------------------------------------------------------
// ReadRange / FileReadCache — per-turn file dedup types
// ---------------------------------------------------------------------------

/// A read range specifying the line offset (1-indexed) and optional
/// line limit used in a single Read call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRange {
    /// 1-indexed starting line number.
    pub offset: usize,
    /// Maximum number of lines to read (None means no limit).
    pub limit: Option<usize>,
}

/// Cached record of a file read within the current turn.

/// Contains the mtime observed at read time and every range that has
/// been read so far. Used for per-turn dedup: if the same path +
/// range is requested again while mtime is unchanged, the tool
/// returns a short "unchanged" hint instead of re-reading.
#[derive(Debug, Clone, PartialEq)]
pub struct FileReadCache {
    /// The mtime recorded when the file was last read.
    pub mtime: Option<SystemTime>,
    /// All ranges that have been read from this file in the current turn.
    pub ranges: Vec<ReadRange>,
}

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
// ToolProgress — incremental execution progress
// ---------------------------------------------------------------------------

/// Real-time progress snapshot for a running tool call.
///
/// Sent periodically by tools (e.g. BashTool) during foreground
/// execution so the UI can display live output statistics.
#[derive(Debug, Clone)]
pub struct ToolProgress {
    /// Number of lines output so far.
    pub lines: usize,
    /// Number of bytes output so far.
    pub bytes: usize,
    /// Elapsed time since command started.
    pub elapsed: Duration,
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
    /// Called after `register_tool_call` and `deregister_tool_call`
    /// so that crash recovery can detect in-flight operations.
    ///
    /// Returns `Ok(())` on success, or `Err` if persistence fails.
    /// Callers that do not require crash-recovery durability
    /// (e.g. register/deregister) may log and continue on error.
    async fn persist_pending_checkpoint(&self) -> Result<(), String> {
        Ok(())
    }

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

    /// Returns whether any child session is currently running.
    ///
    /// Used by the gateway to determine the `child_active` dimension
    /// of session liveness (see `docs/design/session/session-lifecycle.md`).
    fn has_running_child(&self) -> bool {
        false
    }

    /// Record the mtime of a file after it has been read.
    ///
    /// Called by `ReadTool` so that subsequent `EditTool` / `WriteTool`
    /// invocations can verify the file has not been modified externally.
    async fn record_file_read(&self, _path: &str, _mtime: Option<SystemTime>) {}

    /// Retrieve the mtime that was recorded when the file was last read.
    ///
    /// Returns `None` if the file has never been read in this session.
    fn get_file_mtime(&self, _path: &str) -> Option<SystemTime> {
        None
    }

    /// Retrieve the per-turn read cache for a file.
    ///
    /// Returns the recorded mtime and all ranges read so far in the
    /// current turn. Returns `None` if the file has never been read.
    fn get_file_read_cache(&self, _path: &str) -> Option<FileReadCache> {
        None
    }

    /// Record a file read range for per-turn dedup.
    ///
    /// Called by `ReadTool` after a successful read so that subsequent
    /// identical reads (same path + range + mtime) can be short-circuited.
    async fn record_file_read_range(
        &self,
        _path: &str,
        _mtime: Option<SystemTime>,
        _range: ReadRange,
    ) {
    }

    /// Report real-time progress for a running tool call.
    ///
    /// Called periodically during foreground execution to provide the UI
    /// with incremental output statistics. The default implementation
    /// is a no-op (progress is optional).
    async fn report_tool_progress(&self, _call_id: &str, _progress: ToolProgress) {}
}
