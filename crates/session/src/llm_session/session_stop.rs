//! Stop / cascade / kill / cleanup surface for `ConversationSession`.
//!
//! Extracted from `session_handles.rs` (issue #3164): the
//! `stop(cascade, mode)` main path (`stop` / `stop_inner` / the
//! `#[cfg(test)]` `stop_with_kill_budget` seam), the recursive
//! cascade (`cascade_stop_children`), the per-handle kill sweep
//! (`kill_tool_handles`) and the exec-state reset
//! (`clear_exec_state`). The shared [`STOP_KILL_TIMEOUT`] budget
//! and the public stop-related types still live in `session_handles`.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_common::tool_session::KillHandle;
use closeclaw_common::{LlmState, ToolExecState};

use super::session_handles::{CascadeStopInfo, STOP_KILL_TIMEOUT};
use super::ConversationSession;

#[allow(dead_code)]
impl ConversationSession {
    /// Idempotently stop this session and (optionally) all descendant
    /// sessions. See `docs/design/session/session-execution.md` for
    /// the full state table and ordering rules.
    ///
    /// `mode` controls how in-flight operations are terminated:
    /// - **Graceful**: pause external input, cascade with same mode,
    ///   then delegate to `graceful_stop()` (Step 1.2). Cleanup is
    ///   always done after wait completes or times out.
    /// - **Forceful**: cascade with forceful mode, kill tool handles,
    ///   cancel token, then clean up exec state. Order per design doc.
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
        self.stop_inner(cascade, mode, timeout, STOP_KILL_TIMEOUT)
            .await
    }
}

#[allow(dead_code)]
impl ConversationSession {
    /// Internal [`stop`](Self::stop) implementation carrying the
    /// per-handle kill budget.
    ///
    /// Production callers go through `stop()`, which passes the
    /// default [`STOP_KILL_TIMEOUT`]. Tests inject a small budget
    /// through the `#[cfg(test)]`-gated `stop_with_kill_budget` seam
    /// so the budget-expiry path can be exercised with a true
    /// wall-clock assertion ("stop returns within the budget's order
    /// of magnitude") instead of waiting out the production 5 s
    /// budget (STANDARDS §6: no single test case over 5 s).
    async fn stop_inner(
        &self,
        cascade: bool,
        mode: ShutdownMode,
        timeout: Duration,
        kill_budget: Duration,
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
                // Execution order per design doc:
                //   cascade → kill tools → cancel LLM → cleanup
                //
                // Set stopped flag (best-effort, for Gateway
                // is_stopped() check).
                let _ =
                    self.stopped
                        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst);
                // 1. Cascade children with Forceful mode.
                if cascade {
                    self.cascade_stop_children(mode, timeout).await;
                }
                // 2. Kill every registered tool handle.
                self.kill_tool_handles(kill_budget).await;
                // 3. Cancel in-flight LLM requests via cancel token.
                self.cancel_token.cancel();
                // 4. Reset execution state.
                self.clear_exec_state();
                CascadeStopInfo::default()
            }
        }
    }

    /// `#[cfg(test)]`-only test seam: `stop()` with a caller-chosen
    /// per-handle kill budget, forwarding to the private `stop_inner`.
    /// Gated so the budget injection point never enters the production
    /// compile unit (issue #3161 Step 1.4).
    #[cfg(test)]
    pub(super) async fn stop_with_kill_budget(
        &self,
        cascade: bool,
        mode: ShutdownMode,
        timeout: Duration,
        kill_budget: Duration,
    ) -> CascadeStopInfo {
        self.stop_inner(cascade, mode, timeout, kill_budget).await
    }
}

#[allow(dead_code)]
impl ConversationSession {
    /// Recursively call `stop(cascade, mode)` on every live child
    /// session. The mode is forwarded so children are stopped with
    /// the same strategy as the parent.
    /// In Graceful mode, each child's in-flight operations are
    /// waited on (via `graceful_stop`) before the next sibling is
    /// processed. This ensures the cascade follows the design doc
    /// ordering: cascade → wait → cleanup.
    /// Weak references that have been dropped (child completed and reaped) are silently skipped.
    /// Returns [`CascadeStopInfo`] with any children that timed out
    /// during the graceful wait.
    pub fn cascade_stop_children<'a>(
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
}

#[allow(dead_code)]
impl ConversationSession {
    /// Call `kill()` on every registered tool handle, bounded by
    /// `kill_budget` per handle (the stop path passes
    /// [`STOP_KILL_TIMEOUT`]).
    ///
    /// `KillHandle::kill` is synchronous by contract and may block,
    /// so each call is dispatched to the blocking pool: a stalled
    /// kill can no longer freeze the executor task and defeat the
    /// wall-clock budget (see `docs/design/common/core-traits.md`,
    /// KillHandle — the stop path backstops with a wall-clock
    /// budget). When the budget expires, the wait is abandoned with
    /// a warning and stop proceeds; the detached task finishes on
    /// the pool in the background. A `kill()` that never returns
    /// therefore strands its blocking-pool thread until the process
    /// exits — the stranded-thread upper bound is one per registered
    /// handle, i.e. ≤ handle count.
    ///
    /// Before killing, any tool in `RunningForeground` or
    /// `RunningBackground` state is marked `Terminated` so the state
    /// machine reaches a proper terminal state before cleanup.
    pub(super) async fn kill_tool_handles(&self, kill_budget: Duration) {
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
            // Dispatch the synchronous (possibly blocking) kill to
            // the blocking pool so it cannot stall the executor and
            // make the wall-clock budget below unenforceable.
            let kill_task = tokio::task::spawn_blocking(move || handle.kill());
            match tokio::time::timeout(kill_budget, kill_task).await {
                Ok(Ok(Ok(()))) => {}
                Ok(Ok(Err(e))) => {
                    tracing::warn!(
                        call_id = %call_id,
                        error = %e,
                        "kill_tool_handles: handle.kill() returned error"
                    );
                }
                Ok(Err(join_err)) => {
                    // A panic inside `kill()` surfaces as a
                    // `JoinError` on the blocking task; re-raise it
                    // so a panicking kill keeps the pre-existing
                    // "panic propagates out of stop()" semantics.
                    // A shutdown join error (executor going down)
                    // is treated like a failed kill. UT exemption
                    // for the non-panic variant: see the doc of
                    // `test_stop_with_panicking_kill_handle_propagates_panic`
                    // in `tests/stop_kill_path_tests.rs`.
                    if join_err.is_panic() {
                        std::panic::resume_unwind(join_err.into_panic());
                    }
                    tracing::warn!(
                        call_id = %call_id,
                        error = %join_err,
                        "kill_tool_handles: kill task join failed"
                    );
                }
                Err(_elapsed) => {
                    tracing::warn!(
                        call_id = %call_id,
                        "kill_tool_handles: handle.kill() timed out after {:?}",
                        kill_budget
                    );
                }
            }
        }
    }
}

#[allow(dead_code)]
impl ConversationSession {
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

        // tool_handles: drop all entries and decrement busy count
        // for each, so the shutdown drain sees the correct count.
        let tool_count = {
            let map = self
                .tool_handles
                .read()
                .expect("tool_handles lock poisoned");
            map.len()
        };
        self.tool_handles
            .write()
            .expect("tool_handles lock poisoned")
            .clear();
        if let Some(sh) = self.get_shutdown_handle() {
            for _ in 0..tool_count {
                sh.decrement_busy();
            }
        }
        // child_handles: same pattern — decrement busy count for
        // each entry before clearing.
        let child_count = {
            let map = self
                .child_handles
                .read()
                .expect("child_handles lock poisoned");
            map.len()
        };
        self.child_handles
            .write()
            .expect("child_handles lock poisoned")
            .clear();
        if let Some(sh) = self.get_shutdown_handle() {
            for _ in 0..child_count {
                sh.decrement_busy();
            }
        }
    }
}
