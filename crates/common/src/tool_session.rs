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

    /// Returns a reference to the manual backgrounding notify signal.
    ///
    /// Tools can await `signal.notified()` inside `tokio::select!`
    /// to react to user-initiated manual backgrounding requests.
    /// Returns `None` if the session does not support manual
    /// backgrounding (e.g. test doubles).
    fn manual_background_notify(&self) -> Option<Arc<tokio::sync::Notify>> {
        None
    }
}
