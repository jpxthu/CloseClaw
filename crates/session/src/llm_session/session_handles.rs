//! Handle / cancel-token surface for `ConversationSession`.
//!
//! This module holds the stop-path *data* shared across the "how to
//! stop" surface that Step 1.1–1.3 of issue #858 layered on top of
//! the four-dimensional state model implemented in `session_exec`:
//!
//! - [`KillHandle`] — abstract tool-process kill adapter
//! - per-session fields `tool_handles` / `child_handles` /
//!   `cancel_token` / `stopped` (defined on [`ConversationSession`])
//! - register / unregister / inspect methods on
//!   [`ConversationSession`]
//!
//! The stop mechanics themselves live in sibling modules:
//! `session_stop` (stop / cascade / kill / cleanup) and
//! `session_graceful_stop` (graceful wait / force kill).

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use closeclaw_common::canonicalize_or_clone;
use closeclaw_common::{FileReadCache, ReadRange};

use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use super::ConversationSession;

/// Default kill-handle timeout (5 seconds) applied per handle in
/// [`ConversationSession::kill_tool_handles`] and per child session in
/// [`ConversationSession::cascade_stop_children`]. After this, the
/// kill attempt is abandoned and cleanup continues — orphan processes
/// are not allowed to wedge the stop path.
pub(super) const STOP_KILL_TIMEOUT: Duration = Duration::from_secs(5);

/// Default timeout for graceful stop when no explicit timeout is provided
/// by the caller (e.g. `/stop` slash command).
pub const DEFAULT_GRACEFUL_TIMEOUT: Duration = Duration::from_secs(30);

// ── cascade stop info ────────────────────────────────────────────────────

/// Information collected during a cascade stop about children that
/// timed out before completing their graceful stop. Propagated from
/// `cascade_stop_children` → `stop()` → `stop_single_session` so
/// the caller can report progress to the daemon shutdown handler.
#[derive(Debug, Clone, Default)]
pub struct CascadeStopInfo {
    /// Children whose graceful stop timed out: (session_id, elapsed).
    pub timed_out_children: Vec<(String, Duration)>,
}

impl CascadeStopInfo {
    /// Merge another `CascadeStopInfo` into this one.
    pub fn merge(&mut self, other: CascadeStopInfo) {
        self.timed_out_children.extend(other.timed_out_children);
    }
}

// ── graceful stop types ─────────────────────────────────────────────────

/// Progress report emitted by [`ConversationSession::graceful_stop`]
/// when the wait times out. Sent via `progress_tx` so the caller
/// (SessionManager / Daemon) can report what is still running.
#[derive(Debug, Clone)]
pub struct GracefulStopProgress {
    /// Items still in flight at timeout: (name, elapsed since stop began).
    pub waiting_items: Vec<(String, Duration)>,
    /// Number of items still pending.
    pub remaining: usize,
}

/// Outcome of [`ConversationSession::graceful_stop`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GracefulStopResult {
    /// All in-flight operations completed within the timeout.
    Completed,
    /// Forceful escalation detected during the wait.
    Interrupted,
    /// Timeout elapsed; `GracefulStopProgress` was sent via `progress_tx`.
    TimedOut,
}

// ── handle / cancel-token / cascade-stop methods ────────────────────────

#[allow(dead_code)]
impl ConversationSession {
    // ── register / unregister: tool_handles ──────────────────────────────

    /// Register a tool-process kill handle for a given call id.
    ///
    /// Called from `BashTool::execute_command` (foreground path) with a
    /// `crate::tools::builtin::bash_kill::BashKillHandle` wrapping
    /// the spawned `tokio::process::Child`, and (background path)
    /// with a `crate::tools::builtin::bash_kill::BackgroundKillHandle`
    /// pointing at the `BackgroundTaskManager` entry.
    pub fn register_tool_handle(
        &self,
        call_id: impl Into<String>,
        handle: Arc<dyn closeclaw_common::tool_session::KillHandle>,
    ) {
        let id = call_id.into();

        // ── Shutdown gate: reject new tool execution ───────────────────
        if let Some(sh) = self.get_shutdown_handle() {
            if sh.is_shutting_down() {
                tracing::warn!(
                    call_id = %id,
                    "rejecting tool execution: daemon is shutting down"
                );
                return;
            }
        }
        let mut map = self
            .tool_handles
            .write()
            .expect("tool_handles lock poisoned");
        map.insert(id, handle);

        // Increment busy count for drain tracking while tool is running.
        if let Some(sh) = self.get_shutdown_handle() {
            sh.increment_busy();
        }
    }

    /// Remove a previously-registered tool-process kill handle.
    ///
    /// Logs a warning if the id is not registered (the typical
    /// "register-then-unregister" lifecycle makes the warning
    /// actionable — it means the caller double-registered or
    /// double-unregistered).
    pub fn unregister_tool_handle(&self, call_id: &str) {
        let mut map = self
            .tool_handles
            .write()
            .expect("tool_handles lock poisoned");
        if map.remove(call_id).is_none() {
            tracing::warn!(
                call_id = %call_id,
                "unregister_tool_handle: call_id not registered"
            );
        }

        // Decrement busy count for drain tracking when tool exits.
        if let Some(sh) = self.get_shutdown_handle() {
            sh.decrement_busy();
        }
    }

    // ── register / unregister: child_handles ─────────────────────────────

    /// Register a weak reference to a spawned child session.
    ///
    /// Called from `SessionManager::create_child_session` (Step 1.5)
    /// after the child is inserted into `conversation_sessions`. The
    /// weak reference lets `stop(cascade=true)` recursively stop the
    /// child without extending its lifetime.
    pub fn register_child_handle(
        &self,
        child_id: impl Into<String>,
        session: std::sync::Weak<RwLock<ConversationSession>>,
    ) {
        let id = child_id.into();

        // ── Shutdown gate: reject new child session creation ──────────
        if let Some(sh) = self.get_shutdown_handle() {
            if sh.is_shutting_down() {
                tracing::warn!(
                    child_id = %id,
                    "rejecting child session creation: daemon is shutting down"
                );
                return;
            }
        }
        let mut map = self
            .child_handles
            .write()
            .expect("child_handles lock poisoned");
        map.insert(id, session);

        // Increment busy count for drain tracking while child runs.
        if let Some(sh) = self.get_shutdown_handle() {
            sh.increment_busy();
        }
    }

    /// Remove a previously-registered child-session handle.
    pub fn unregister_child_handle(&self, child_id: &str) {
        let mut map = self
            .child_handles
            .write()
            .expect("child_handles lock poisoned");
        if map.remove(child_id).is_none() {
            tracing::warn!(
                child_id = %child_id,
                "unregister_child_handle: child_id not registered"
            );
        }

        // Decrement busy count for drain tracking when child exits.
        if let Some(sh) = self.get_shutdown_handle() {
            sh.decrement_busy();
        }
    }

    // ── cancel-token accessors ──────────────────────────────────────────

    /// Returns a clone of this session's cancellation token.
    ///
    /// Callers that need to `await cancel_token.cancelled()` typically
    /// clone the token (it is `Clone`) and move the clone into the
    /// `tokio::select!` branch. The clone is independent of the
    /// parent/child relationship — both the parent and the child
    /// observe the same cancellation event.
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel_token
    }

    /// Returns a *new* child token derived from this session's token.
    ///
    /// `CancellationToken::child_token()` gives a token that is
    /// automatically cancelled when the parent is cancelled, but a
    /// `cancel()` on the child does NOT propagate to the parent. This
    /// is exactly the semantics `SessionManager::create_child_session`
    /// needs for the parent-child cascade in Step 1.5.
    pub fn child_cancel_token(&self) -> CancellationToken {
        self.cancel_token.child_token()
    }

    /// Returns true if this session's cancellation token has been
    /// signalled (either directly, or because an ancestor in the
    /// parent-child token tree was cancelled).
    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }

    /// Returns true if `stop()` has been called on this session at
    /// least once (idempotency flag). Note: a child whose parent was
    /// cancelled may see `is_cancelled() == true` but
    /// `is_stopped() == false` — the parent has not (and will not)
    /// invoke the child's `stop()` for cleanup; the child must call
    /// its own `stop()` to reap its tool/child handles.
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    // ── manual backgrounding ─────────────────────────────────────────────

    /// Signal all foreground commands to move to background.
    ///
    /// This fires the `manual_background_signal`
    /// [`tokio::sync::Notify`], which
    /// unblocks any `tokio::select!` branch in `BashTool` waiting on
    /// the manual-background path. Each affected foreground command
    /// will be handed off to `bg_manager.backgroundize_task()` and
    /// return a `ToolResult` with `backgroundedByUser: true`.
    ///
    /// The call is idempotent — signalling when no foreground
    /// commands are waiting is a harmless no-op.
    pub fn trigger_manual_background(&self) {
        self.manual_background_signal.notify_waiters();
        tracing::info!(
            session_id = %self.session_id,
            "trigger_manual_background: signal fired"
        );
    }
}

// ── ToolSession trait implementation ────────────────────────────────────

#[async_trait::async_trait]
impl closeclaw_common::tool_session::ToolSession for ConversationSession {
    async fn register_tool_handle(
        &self,
        call_id: String,
        handle: Arc<dyn closeclaw_common::tool_session::KillHandle>,
    ) {
        // Delegate to the inherent method which also handles
        // shutdown-gate and busy-count tracking.
        self.register_tool_handle(call_id, handle);
    }

    fn manual_background_notify(&self) -> Option<Arc<tokio::sync::Notify>> {
        Some(self.manual_background_notify())
    }

    fn enter_waiting(&self) {
        ConversationSession::enter_waiting(self);
    }

    fn exit_waiting(&self) {
        ConversationSession::exit_waiting(self);
    }

    fn is_waiting(&self) -> bool {
        ConversationSession::is_waiting(self)
    }

    async fn register_tool_call(&self, call_id: String, tool_name: String, args_summary: String) {
        ConversationSession::register_tool_call(self, call_id, tool_name, args_summary);
        if let Err(e) = self.persist_pending_checkpoint().await {
            tracing::warn!(
                session_id = %self.session_id,
                "register_tool_call: checkpoint persist failed: {}",
                e
            );
        }
    }

    async fn deregister_tool_call(&self, call_id: String) {
        ConversationSession::deregister_tool_call(self, &call_id);
        if let Err(e) = self.persist_pending_checkpoint().await {
            tracing::warn!(
                session_id = %self.session_id,
                "deregister_tool_call: checkpoint persist failed: {}",
                e
            );
        }
    }

    async fn update_tool_state(&self, call_id: &str, state: closeclaw_common::ToolExecState) {
        ConversationSession::update_tool_state(self, call_id, state);
    }

    async fn report_tool_progress(&self, call_id: &str, progress: closeclaw_common::ToolProgress) {
        tracing::debug!(
            call_id = %call_id,
            lines = progress.lines,
            bytes = progress.bytes,
            elapsed_ms = progress.elapsed.as_millis() as u64,
            "tool progress"
        );
    }

    async fn register_child_state(&self, child_id: String, agent_id: String, task_summary: String) {
        ConversationSession::register_child(self, child_id, agent_id, task_summary);
        if let Err(e) = self.persist_pending_checkpoint().await {
            tracing::warn!(
                session_id = %self.session_id,
                "register_child_state: checkpoint persist failed: {}",
                e
            );
        }
    }

    async fn deregister_child_state(&self, child_id: String) {
        ConversationSession::deregister_child(self, &child_id);
        if let Err(e) = self.persist_pending_checkpoint().await {
            tracing::warn!(
                session_id = %self.session_id,
                "deregister_child_state: checkpoint persist failed: {}",
                e
            );
        }
    }

    fn has_running_child(&self) -> bool {
        ConversationSession::has_running_child(self)
    }

    async fn record_file_read(&self, path: &str, mtime: Option<SystemTime>) {
        let canonical = canonicalize_or_clone(Path::new(path));
        let mut map = self.file_mtimes.write().expect("file_mtimes lock poisoned");
        match mtime {
            Some(t) => {
                map.insert(canonical, t);
            }
            None => {
                map.remove(&canonical);
            }
        }
    }

    fn get_file_mtime(&self, path: &str) -> Option<SystemTime> {
        let canonical = canonicalize_or_clone(Path::new(path));
        let map = self.file_mtimes.read().expect("file_mtimes lock poisoned");
        map.get(&canonical).copied()
    }

    fn get_file_read_cache(&self, path: &str) -> Option<FileReadCache> {
        let canonical = canonicalize_or_clone(Path::new(path));
        let map = self
            .file_read_ranges
            .read()
            .expect("file_read_ranges lock poisoned");
        map.get(&canonical).cloned()
    }

    async fn record_file_read_range(
        &self,
        path: &str,
        mtime: Option<SystemTime>,
        range: ReadRange,
    ) {
        let canonical = canonicalize_or_clone(Path::new(path));
        let mut map = self
            .file_read_ranges
            .write()
            .expect("file_read_ranges lock poisoned");
        let entry = map.entry(canonical).or_insert_with(|| FileReadCache {
            mtime,
            ranges: Vec::new(),
        });
        entry.mtime = mtime;
        entry.ranges.push(range);
    }

    async fn persist_pending_checkpoint(&self) -> Result<(), String> {
        let Some(ref storage) = self.checkpoint_storage else {
            return Ok(());
        };
        let session_id = self.session_id.clone();
        let collected_ops = self.collect_pending_operations();
        let system_appends = self.user_system_appends().to_vec();
        let verbosity = self.verbosity_level();
        let workflow_run = self.workflow_run_value();
        // Load existing checkpoint or create a minimal one.
        let mut cp = match storage.load_checkpoint(&session_id).await {
            Ok(Some(cp)) => cp,
            _ => crate::persistence::SessionCheckpoint::new(session_id),
        };
        // Merge pending operations: replace ToolCall/SubSessionSpawn ops
        // (collected from live session state) while preserving OutboundMessage
        // ops (managed at checkpoint layer via write-ahead / ack-clear).
        // This prevents write-ahead OutboundMessage ops from being overwritten
        // by the live-state collector which has no outbound queue.
        cp.pending_operations.retain(|op| {
            matches!(
                op.op_type,
                crate::persistence::PendingOperationType::OutboundMessage
            )
        });
        cp.pending_operations.extend(collected_ops);
        cp.user_appends = system_appends;
        cp.verbosity_level = verbosity;
        cp.workflow_run = workflow_run;
        cp.touch();
        cp.last_message_at = Some(chrono::Utc::now());
        storage.save_checkpoint(&cp).await.map_err(|e| {
            let msg = format!("persist_pending_checkpoint: save failed: {}", e);
            tracing::warn!(session_id = %cp.session_id, "{}", msg);
            msg
        })
    }
}
