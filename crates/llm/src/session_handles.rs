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

use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use closeclaw_common::tool_session::KillHandle;

use super::session::ConversationSession;
use super::session_state::LlmState;

/// Default kill-handle timeout (5 seconds) applied per handle in
/// [`ConversationSession::kill_tool_handles`] and per child session in
/// [`ConversationSession::cascade_stop_children`]. After this, the
/// kill attempt is abandoned and cleanup continues — orphan processes
/// are not allowed to wedge the stop path.
const STOP_KILL_TIMEOUT: Duration = Duration::from_secs(5);

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

    // ── stop(cascade) ───────────────────────────────────────────────────

    /// Idempotently stop this session and (optionally) all descendant
    /// sessions. See `docs/design/session/session-execution.md` for
    /// the full state table and ordering rules.
    ///
    /// Idempotency: subsequent calls are no-ops once `stopped` has
    /// been set, so the caller does not need to guard.
    pub async fn stop(&self, cascade: bool) {
        // 1. Idempotency check. `swap` returns the previous value —
        // if it was already true, this is a duplicate stop.
        if self
            .stopped
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        // 2. Fire the cancel token first. This unblocks any
        //    `tokio::select!` branch in `call_llm` /
        //    `call_llm_streaming` waiting on `cancel_token.cancelled()`
        //    without us having to touch `llm_state` directly. The
        //    token also propagates to every child token derived from
        //    this one (via `child_cancel_token()`), so child LLM
        //    requests are cancelled in lockstep.
        self.cancel_token.cancel();

        // 3. If cascading, recurse into every live child session.
        //    A child's own `stopped` flag may still be false even
        //    though its cancel token is now triggered — that is
        //    expected. Each child is responsible for its own
        //    `tool_handles` / `child_handles` cleanup, which only
        //    `stop()` performs.
        if cascade {
            self.cascade_stop_children().await;
        }

        // 4. Kill every registered tool handle. Each kill is wrapped
        //    in a 5s timeout so a wedged process cannot block the
        //    cascade indefinitely.
        self.kill_tool_handles().await;

        // 5. Reset execution state so the session returns to a
        //    well-defined idle surface. This is the "兜底" cleanup
        //    — LLM requests should already have unwound via the
        //    cancel-token branch, but we still want tool_states and
        //    child_states to reflect "nothing in flight" once stop
        //    returns.
        self.clear_exec_state();
    }

    /// Recursively call `stop(true)` on every live child session.
    /// Weak references that have been dropped (child completed and
    /// reaped) are silently skipped.
    fn cascade_stop_children<'a>(
        &'a self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            // Snapshot the children so we don't hold the lock across
            // the (potentially long-running) recursive stop calls.
            let snapshot: Vec<std::sync::Weak<RwLock<ConversationSession>>> = {
                let map = self
                    .child_handles
                    .read()
                    .expect("child_handles lock poisoned");
                map.values().cloned().collect()
            };

            for weak in snapshot {
                let Some(child_arc) = weak.upgrade() else {
                    // Child already dropped — nothing to do.
                    continue;
                };
                // Per-child wall-clock budget. If the child's own stop
                // path hangs (e.g. a kill handle that ignores timeouts),
                // we abandon the wait and move on. The orphan process
                // is a known-acceptable cost; the cascade must make
                // progress.
                let stop_fut = async {
                    let guard = child_arc.read().await;
                    guard.stop(true).await;
                };
                if tokio::time::timeout(STOP_KILL_TIMEOUT, stop_fut)
                    .await
                    .is_err()
                {
                    tracing::warn!(
                        child = ?child_arc,
                        "cascade_stop_children: child stop() timed out after {:?}",
                        STOP_KILL_TIMEOUT
                    );
                }
            }
        })
    }

    /// Call `kill()` on every registered tool handle, bounded by
    /// [`STOP_KILL_TIMEOUT`] per handle.
    async fn kill_tool_handles(&self) {
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
    fn clear_exec_state(&self) {
        // llm_state → Idle. The cancel-token branch in call_llm
        // should already have set this, but we make it explicit
        // so post-stop invariants hold even if a future caller
        // routes around the cancel token.
        *self.llm_state.write().expect("llm_state lock poisoned") = LlmState::Idle;

        // tool_states and child_states: drop everything. We don't
        // need to mark them as Terminated because the session is
        // about to either be re-used (with fresh state from
        // `replace_messages` / new turns) or be torn down.
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
}
