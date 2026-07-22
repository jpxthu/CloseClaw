//! Handle / cancel-token / cascade-stop surface for `ConversationSession`.
//!
//! This module holds the cross-cutting "how to stop" surface that
//! Step 1.1–1.3 of issue #858 layered on top of the three-dimensional
//! state model implemented in [`super::session_exec`]:
//!
//! - [`KillHandle`] — abstract tool-process kill adapter (the only
//!   public trait in this module)
//! - per-session fields `tool_handles` / `child_handles` /
//!   `cancel_token` / `stopped` (defined in [`super::session`])
//! - register / unregister / inspect / stop methods on
//!   [`ConversationSession`]
//!
//! `stop(cascade)` is split into three small helpers
//! ([`ConversationSession::cascade_stop_children`],
//! [`ConversationSession::kill_tool_handles`],
//! [`ConversationSession::clear_exec_state`]) so no single function
//! crosses the CONTRIBUTING.md 50-line soft cap.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_common::tool_session::KillHandle;
use closeclaw_common::ToolExecState;

use super::ConversationSession;
use closeclaw_common::LlmState;

/// Default kill-handle timeout (5 seconds) applied per handle in
/// [`ConversationSession::kill_tool_handles`] and per child session in
/// [`ConversationSession::cascade_stop_children`]. After this, the
/// kill attempt is abandoned and cleanup continues — orphan processes
/// are not allowed to wedge the stop path.
const STOP_KILL_TIMEOUT: Duration = Duration::from_secs(5);

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
    /// [`crate::tools::builtin::bash_kill::BashKillHandle`] wrapping
    /// the spawned `tokio::process::Child`, and (background path)
    /// with a [`crate::tools::builtin::bash_kill::BackgroundKillHandle`]
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
        let mut map = self
            .child_handles
            .write()
            .expect("child_handles lock poisoned");
        map.insert(id, session);
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
    /// This fires the `manual_background_signal` [`Notify`], which
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

    // ── force_kill ──────────────────────────────────────────────────────

    /// Forcefully kill tool processes and cancel in-flight LLM requests.
    ///
    /// Used by the forceful-shutdown path in
    /// [`SessionManager::stop_single_session`] to immediately terminate
    /// running tool processes and cancel ongoing LLM streams without
    /// clearing execution state. This allows
    /// [`collect_pending_operations`](Self::collect_pending_operations) to
    /// still observe tool_states / child_states for checkpoint recording.
    ///
    /// Incomplete assistant message fragments are discarded: the cancel
    /// token causes in-flight streaming LLM calls to return
    /// `LLMError::Cancelled`, and the Gateway layer does not append
    /// partial assistant messages to the conversation history.
    ///
    /// This method is idempotent — cancelling an already-cancelled token
    /// and killing already-cleared handles are harmless no-ops.
    pub async fn force_kill(&self) {
        // Cancel in-flight LLM requests. Streaming calls observe this
        // via `cancel_token.cancelled()` in the Gateway's
        // `call_llm_streaming` select branch and return Cancelled.
        self.cancel_token.cancel();

        // Kill every registered tool process.
        self.kill_tool_handles().await;

        tracing::info!(
            session_id = %self.session_id,
            "force_kill: tool processes killed, LLM requests cancelled"
        );
    }

    // ── graceful_stop ────────────────────────────────────────────────────

    /// Wait for in-flight operations to complete, with a hard timeout.
    ///
    /// Polls every 100 ms for LLM streaming and tool execution state.
    /// Checks for forceful escalation via the session's shutdown handle.
    /// On timeout, sends a [`GracefulStopProgress`] report via
    /// `progress_tx` and returns [`GracefulStopResult::TimedOut`].
    ///
    /// This method does **not** clean up or persist state — that is
    /// the responsibility of `stop()` (cleanup) and the caller
    /// (`stop_single_session` in SessionManager, persistence).
    pub async fn graceful_stop(
        &self,
        timeout: Duration,
        progress_tx: Option<mpsc::Sender<GracefulStopProgress>>,
    ) -> GracefulStopResult {
        let start = tokio::time::Instant::now();
        let mut streaming_seen = false;

        loop {
            // ── forceful escalation check ────────────────────────────
            if let Some(sh) = self.get_shutdown_handle() {
                if sh.is_forceful() {
                    tracing::info!(
                        session_id = %self.session_id,
                        "graceful_stop: interrupted by forceful escalation"
                    );
                    return GracefulStopResult::Interrupted;
                }
            }

            // ── timeout check ────────────────────────────────────────
            if start.elapsed() >= timeout {
                let waiting = self.collect_waiting_items(start);
                let remaining = waiting.len();
                if let Some(tx) = progress_tx {
                    let _ = tx
                        .send(GracefulStopProgress {
                            waiting_items: waiting,
                            remaining,
                        })
                        .await;
                }
                return GracefulStopResult::TimedOut;
            }

            // ── state inspection ─────────────────────────────────────
            let state = self.llm_state();
            let is_streaming = matches!(state, LlmState::Receiving | LlmState::Requesting);
            let has_running_tools = self.has_running_tools();

            if is_streaming {
                streaming_seen = true;
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }

            if streaming_seen {
                let pending = self.extract_pending_tool_calls();
                if !pending.is_empty() {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                if !has_running_tools {
                    return GracefulStopResult::Completed;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }

            // Not streaming — idle path.
            if !has_running_tools {
                return GracefulStopResult::Completed;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Collect the names and elapsed times of items still in flight.
    fn collect_waiting_items(&self, start: tokio::time::Instant) -> Vec<(String, Duration)> {
        let elapsed = start.elapsed();
        let mut items = Vec::new();

        let state = self.llm_state();
        if matches!(state, LlmState::Receiving | LlmState::Requesting) {
            items.push(("llm_stream".to_string(), elapsed));
        }

        let tool_states = self.tool_states.read().expect("tool_states lock poisoned");
        for (id, (state, _)) in tool_states.iter() {
            if matches!(
                state,
                ToolExecState::RunningForeground | ToolExecState::RunningBackground
            ) {
                items.push((id.clone(), elapsed));
            }
        }
        items
    }

    /// Returns `true` if any tool is in a running state.
    fn has_running_tools(&self) -> bool {
        let tool_states = self.tool_states.read().expect("tool_states lock poisoned");
        tool_states.values().any(|(s, _)| {
            matches!(
                s,
                ToolExecState::RunningForeground | ToolExecState::RunningBackground
            )
        })
    }

    // ── stop(cascade, mode) ─────────────────────────────────────────────

    /// Idempotently stop this session and (optionally) all descendant
    /// sessions. See `docs/design/session/session-execution.md` for
    /// the full state table and ordering rules.
    ///
    /// `mode` controls how in-flight operations are terminated:
    /// - **Graceful**: pause external input, cascade with same mode,
    ///   then delegate to `graceful_stop()` (Step 1.2). Cleanup is
    ///   always done after wait completes or times out.
    /// - **Forceful**: cancel token, cascade with forceful mode, kill
    ///   tool handles immediately. Existing behavior preserved.
    ///
    /// `timeout` is the overall time budget for the cascade stop.
    /// In **Graceful** mode it is shared across all child sessions
    /// via a deadline — no child can exceed the remaining budget.
    /// In **Forceful** mode it should be `Duration::ZERO` since
    /// kills are immediate (no waiting).
    ///
    /// Idempotency: subsequent calls are no-ops once `stopped` has
    /// been set, so the caller does not need to guard.
    ///
    /// Returns [`CascadeStopInfo`] with any timed-out children from
    /// the cascade.
    pub async fn stop(
        &self,
        cascade: bool,
        mode: ShutdownMode,
        timeout: Duration,
    ) -> CascadeStopInfo {
        match mode {
            ShutdownMode::Graceful => {
                // Cascade children with the same Graceful mode,
                // waiting for each child's in-flight operations
                // before moving to the next sibling. This follows
                // the design doc order: cascade → wait → cleanup.
                //
                // Set the stopped flag so callers that invoke stop()
                // directly (without stop_single_session) see the
                // flag set. This is idempotent and safe to repeat
                // during recursive cascade.
                let _ =
                    self.stopped
                        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst);
                let mut cascade_info = CascadeStopInfo::default();
                if cascade {
                    cascade_info = self.cascade_stop_children(mode, timeout).await;
                }
                // NOTE: clear_exec_state() is intentionally NOT called
                // here. The caller (stop_single_session) needs the
                // exec state to remain intact for graceful_wait() to
                // observe in-flight operations. The caller calls
                // clear_exec_state() after graceful_wait() completes
                // (Completed path) or defers it on TimedOut/Interrupted.
                cascade_info
            }
            ShutdownMode::Forceful => {
                // Forceful path: always execute (idempotent ops).
                // No stopped-flag check — operations are safe to
                // repeat (token cancel, cascade, kill, clear are
                // all idempotent). This allows stop() to be called
                // from both stop_single_session (stopped already
                // set) and kill_child (stopped not set).
                //
                // Set stopped flag (best-effort, for Gateway
                // is_stopped() check).
                let _ =
                    self.stopped
                        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst);
                // Cancel in-flight LLM requests via cancel token.
                self.cancel_token.cancel();
                // Cascade children with Forceful mode.
                if cascade {
                    self.cascade_stop_children(mode, timeout).await;
                }
                // Kill every registered tool handle.
                self.kill_tool_handles().await;
                // Reset execution state.
                self.clear_exec_state();
                CascadeStopInfo::default()
            }
        }
    }

    /// Recursively call `stop(cascade, mode)` on every live child
    /// session. The mode is forwarded so children are stopped with
    /// the same strategy as the parent.
    ///
    /// In Graceful mode, each child's in-flight operations are
    /// waited on (via `graceful_stop`) before the next sibling is
    /// processed. This ensures the cascade follows the design doc
    /// ordering: cascade → wait → cleanup.
    ///
    /// Weak references that have been dropped (child completed and
    /// reaped) are silently skipped.
    ///
    /// Returns [`CascadeStopInfo`] with any children that timed out
    /// during the graceful wait.
    fn cascade_stop_children<'a>(
        &'a self,
        mode: ShutdownMode,
        overall_timeout: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CascadeStopInfo> + Send + 'a>> {
        Box::pin(async move {
            let mut info = CascadeStopInfo::default();

            // Snapshot the children so we don't hold the lock across
            // the (potentially long-running) recursive stop calls.
            let snapshot: Vec<std::sync::Weak<RwLock<ConversationSession>>> = {
                let map = self
                    .child_handles
                    .read()
                    .expect("child_handles lock poisoned");
                map.values().cloned().collect()
            };

            // Shared deadline across all children.
            let deadline = tokio::time::Instant::now() + overall_timeout;

            for weak in snapshot {
                // Compute remaining budget. When overall_timeout is
                // Duration::ZERO the deadline equals now; use direct
                // comparison so Duration::ZERO yields 0 remaining time
                // (children are still cascade-stopped with zero timeout)
                // instead of breaking the loop.
                let now = tokio::time::Instant::now();
                let remaining_time = if now >= deadline {
                    Duration::ZERO
                } else {
                    deadline - now
                };

                let Some(child_arc) = weak.upgrade() else {
                    // Child already dropped — nothing to do.
                    continue;
                };
                // Per-child wall-clock budget. Use the minimum of
                // the remaining time in the overall budget and
                // STOP_KILL_TIMEOUT so no single child can exceed
                // either limit.
                let per_child_timeout = remaining_time.min(STOP_KILL_TIMEOUT);
                let child_start = tokio::time::Instant::now();
                let stop_fut = async {
                    let guard = child_arc.read().await;
                    // In Graceful mode, wait for the child's in-flight
                    // operations BEFORE calling stop() on it. This
                    // ensures the child's tools/LLM stream complete
                    // (or time out) before we proceed to the next
                    // sibling.
                    if mode == ShutdownMode::Graceful {
                        let _ = guard.graceful_stop(per_child_timeout, None).await;
                    }
                    guard.stop(true, mode, per_child_timeout).await;
                };
                let timed_out = if per_child_timeout.is_zero() {
                    // Duration::ZERO: tokio::time::timeout fires
                    // immediately at the first .await inside stop_fut,
                    // so the child's stop() would never actually run.
                    // Skip the timeout wrapper and await directly.
                    let _ = stop_fut.await;
                    false
                } else {
                    tokio::time::timeout(per_child_timeout, stop_fut)
                        .await
                        .is_err()
                };
                if timed_out {
                    let elapsed = child_start.elapsed();
                    tracing::warn!(
                        child = ?child_arc,
                        elapsed = ?elapsed,
                        "cascade_stop_children: child stop() timed out after {:?}",
                        per_child_timeout
                    );
                    // Collect timeout info for the caller.
                    let child_id = {
                        let guard = child_arc.read().await;
                        guard.session_id.clone()
                    };
                    info.timed_out_children.push((child_id, elapsed));
                }
            }

            info
        })
    }

    /// Call `kill()` on every registered tool handle, bounded by
    /// [`STOP_KILL_TIMEOUT`] per handle.
    ///
    /// Before killing, any tool in `RunningForeground` or
    /// `RunningBackground` state is marked `Terminated` so the state
    /// machine reaches a proper terminal state before cleanup.
    async fn kill_tool_handles(&self) {
        // Set Terminated on all Running tools before killing.
        // This ensures the state machine reaches a terminal state
        // before the process is reaped and clear_exec_state runs.
        {
            let mut states = self.tool_states.write().expect("tool_states lock poisoned");
            for (state, _) in states.values_mut() {
                if matches!(
                    *state,
                    ToolExecState::RunningForeground | ToolExecState::RunningBackground
                ) {
                    *state = ToolExecState::Terminated;
                }
            }
        }

        // Snapshot the handle set under the lock, then drop the
        // lock before the (blocking / IO) kill calls.
        let snapshot: Vec<(String, Arc<dyn KillHandle>)> = {
            let map = self
                .tool_handles
                .read()
                .expect("tool_handles lock poisoned");
            map.iter()
                .map(|(k, v)| (k.clone(), Arc::clone(v)))
                .collect()
        };

        for (call_id, handle) in snapshot {
            let kill_fut = async { handle.kill() };
            match tokio::time::timeout(STOP_KILL_TIMEOUT, kill_fut).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    tracing::warn!(
                        call_id = %call_id,
                        error = %e,
                        "kill_tool_handles: handle.kill() returned error"
                    );
                }
                Err(_elapsed) => {
                    tracing::warn!(
                        call_id = %call_id,
                        "kill_tool_handles: handle.kill() timed out after {:?}",
                        STOP_KILL_TIMEOUT
                    );
                }
            }
        }
    }

    /// Reset llm_state / tool_states / child_states to a clean
    /// "nothing in flight" surface and clear the in-memory handle
    /// maps. Called at the end of `stop()`.
    pub fn clear_exec_state(&self) {
        // llm_state → Idle. The cancel-token branch in call_llm
        // should already have set this, but we make it explicit
        // so post-stop invariants hold even if a future caller
        // routes around the cancel token.
        *self.llm_state.write().expect("llm_state lock poisoned") = LlmState::Idle;

        // tool_states and child_states: drop everything. We don't
        // need to mark them as Terminated because the session is
        // about to either be re-used (with fresh state from
        // `apply_transcript_op` / new turns) or be torn down.
        self.tool_states
            .write()
            .expect("tool_states lock poisoned")
            .clear();
        self.child_states
            .write()
            .expect("child_states lock poisoned")
            .clear();

        // tool_handles and child_handles: drop all entries. The
        // Arc/Weak counts on the processes/sessions go to zero
        // here (assuming no other holders), which lets the
        // underlying process exit and the child session be
        // reaped.
        self.tool_handles
            .write()
            .expect("tool_handles lock poisoned")
            .clear();
        self.child_handles
            .write()
            .expect("child_handles lock poisoned")
            .clear();
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
        self.persist_pending_checkpoint().await;
    }

    async fn deregister_tool_call(&self, call_id: String) {
        ConversationSession::deregister_tool_call(self, &call_id);
        self.persist_pending_checkpoint().await;
    }

    async fn update_tool_state(&self, call_id: &str, state: closeclaw_common::ToolExecState) {
        ConversationSession::update_tool_state(self, call_id, state);
    }

    async fn register_child_state(&self, child_id: String, agent_id: String, task_summary: String) {
        ConversationSession::register_child(self, child_id, agent_id, task_summary);
        self.persist_pending_checkpoint().await;
    }

    async fn deregister_child_state(&self, child_id: String) {
        ConversationSession::deregister_child(self, &child_id);
        self.persist_pending_checkpoint().await;
    }

    async fn persist_pending_checkpoint(&self) {
        let Some(ref storage) = self.checkpoint_storage else {
            return;
        };
        let session_id = self.session_id.clone();
        let pending_ops = self.collect_pending_operations();
        let system_appends = self.user_system_appends().to_vec();
        let verbosity = self.verbosity_level();
        let storage = Arc::clone(storage);
        tokio::spawn(async move {
            // Load existing checkpoint or create a minimal one.
            let cp = match storage.load_checkpoint(&session_id).await {
                Ok(Some(mut cp)) => {
                    cp.pending_operations = pending_ops;
                    cp.system_appends = system_appends;
                    cp.verbosity_level = verbosity;
                    cp.touch();
                    cp
                }
                _ => {
                    let mut cp = crate::persistence::SessionCheckpoint::new(session_id.clone());
                    cp.pending_operations = pending_ops;
                    cp.system_appends = system_appends;
                    cp.verbosity_level = verbosity;
                    cp
                }
            };
            if let Err(e) = storage.save_checkpoint(&cp).await {
                tracing::warn!(
                    session_id = %cp.session_id,
                    "persist_pending_checkpoint: save failed: {}",
                    e
                );
            }
        });
    }
}
