//! Leaf-to-root session shutdown for `SessionManager`.
//!
//! Implements the hierarchical stop order required by the phase-based
//! daemon shutdown design doc:
//!
//! 1. Build a parent → children tree from the `children` tracking table.
//! 2. BFS from roots to leaves, then reverse — stops leaves first,
//!    parents last.  Same-level sessions stop concurrently.
//! 3. Per-session behaviour depends on [`ShutdownMode`]:
//!    - **Graceful**: wait for in-flight ops to finish with a timeout.
//!      On timeout, report waiting items without killing.
//!    - **Forceful**: stop immediately, persist checkpoint.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_session::llm_session::session_handles::GracefulStopProgress;

use super::SessionManager;

// ── graceful stop outcome ───────────────────────────────────────────────

/// Outcome of stopping a single session gracefully.
///
/// The caller uses this to decide the next action:
/// - `Completed` — session stopped successfully.
/// - `TimedOut` — graceful wait timed out; caller decides whether to
///   retry with forceful, continue waiting, or abandon.
/// - `Interrupted` — forceful escalation was detected during the
///   graceful wait; session was force-stopped internally.
#[derive(Debug, Clone)]
pub(crate) enum GracefulStopOutcome {
    /// All in-flight operations completed within the timeout.
    Completed,
    /// Graceful wait timed out. The contained progress information
    /// describes what is still in flight so the caller can report it
    /// or decide on escalation.
    TimedOut {
        /// Items still waiting at timeout: (name, elapsed since stop
        /// began).
        waiting_items: Vec<(String, Duration)>,
        /// Number of items still pending.
        remaining: usize,
    },
    /// Forceful escalation was detected during the graceful wait;
    /// session was force-stopped internally.
    Interrupted,
}

// ── progress reporting ──────────────────────────────────────────────────

/// Progress event emitted by [`SessionManager::stop_all_sessions`]
/// each time a single session stop completes.
#[derive(Debug, Clone)]
pub struct StopProgress {
    pub session_id: String,
    pub success: bool,
    pub remaining: usize,
}

/// Aggregated result of stopping all sessions.
#[derive(Debug, Default)]
pub struct StopResult {
    pub succeeded: usize,
    pub failed: usize,
    pub skipped: usize,
}

impl StopResult {
    /// Total number of sessions processed.
    pub fn total(&self) -> usize {
        self.succeeded + self.failed + self.skipped
    }
}

// ── public API ──────────────────────────────────────────────────────────

impl SessionManager {
    /// Stop all active sessions in leaf-to-root order.
    /// Siblings at the same level stop concurrently.
    pub async fn stop_all_sessions(
        &self,
        mode: ShutdownMode,
        timeout: Duration,
        progress_tx: Option<&tokio::sync::mpsc::Sender<StopProgress>>,
    ) -> StopResult {
        let tree = self.build_stop_tree().await;
        let stop_order = stop_order_from_tree(&tree);
        if stop_order.is_empty() {
            tracing::info!("stop_all_sessions: no active sessions");
            return StopResult::default();
        }
        let total_sessions: usize = stop_order.iter().map(|l| l.len()).sum();
        tracing::info!(
            count = total_sessions,
            mode = ?mode,
            "stop_all_sessions: starting leaf-to-root shutdown"
        );
        let mut result = StopResult::default();
        let mut processed = 0usize;
        for level in &stop_order {
            let effective_mode = self.resolve_effective_mode(mode).await;
            let count = self
                .process_stop_level(
                    level,
                    effective_mode,
                    timeout,
                    &mut result,
                    progress_tx,
                    total_sessions.saturating_sub(processed),
                )
                .await;
            processed += count;
        }
        tracing::info!(
            succeeded = result.succeeded,
            failed = result.failed,
            skipped = result.skipped,
            "stop_all_sessions: complete"
        );
        result
    }

    /// Process a single level of sessions concurrently.
    /// Returns count of sessions processed.
    ///
    /// On graceful timeout, escalates to forceful stop for the
    /// timed-out session (daemon shutdown behavior).
    async fn process_stop_level(
        &self,
        level: &[String],
        mode: ShutdownMode,
        timeout: Duration,
        result: &mut StopResult,
        progress_tx: Option<&tokio::sync::mpsc::Sender<StopProgress>>,
        remaining: usize,
    ) -> usize {
        let futures: Vec<_> = level
            .iter()
            .map(|sid| self.stop_single_session(sid, mode, false, timeout, None))
            .collect();

        let outcomes = futures::future::join_all(futures).await;
        let count = level.len();
        for (idx, outcome) in outcomes.into_iter().enumerate() {
            // On graceful timeout, escalate to forceful for daemon
            // shutdown — same behaviour as before this step.
            let effective_outcome = match outcome {
                Ok(GracefulStopOutcome::TimedOut { remaining: r, .. })
                    if mode == ShutdownMode::Graceful =>
                {
                    tracing::info!(
                        session_id = %level[idx],
                        remaining = r,
                        "graceful timeout during level stop, \
                         escalating to forceful"
                    );
                    self.stop_single_session(
                        &level[idx],
                        ShutdownMode::Forceful,
                        false,
                        Duration::ZERO,
                        None,
                    )
                    .await
                }
                other => other,
            };
            let success = Self::process_stop_outcome(effective_outcome, result);
            Self::notify_stop_progress(
                progress_tx,
                &level[idx],
                success,
                remaining.saturating_sub(idx + 1),
            )
            .await;
        }
        count
    }

    /// Send a stop progress event if a progress channel is provided.
    async fn notify_stop_progress(
        progress_tx: Option<&tokio::sync::mpsc::Sender<StopProgress>>,
        session_id: &str,
        success: bool,
        remaining: usize,
    ) {
        if let Some(tx) = progress_tx {
            let _ = tx
                .send(StopProgress {
                    session_id: session_id.to_string(),
                    success,
                    remaining,
                })
                .await;
        }
    }

    /// Record a stop outcome and return success flag.
    fn process_stop_outcome(
        outcome: Result<GracefulStopOutcome, StopError>,
        result: &mut StopResult,
    ) -> bool {
        match outcome {
            Ok(GracefulStopOutcome::Completed) | Ok(GracefulStopOutcome::Interrupted) => {
                result.succeeded += 1;
                true
            }
            Ok(GracefulStopOutcome::TimedOut { .. }) => {
                result.failed += 1;
                false
            }
            Err(StopError::Skipped) => {
                result.skipped += 1;
                false
            }
            Err(StopError::Failed) => {
                result.failed += 1;
                false
            }
        }
    }
}

// ── tree construction ───────────────────────────────────────────────────

/// Internal session tree built from the `children` tracking table.
/// Maps parent_session_id → list of child session_ids.
type SessionTree = HashMap<String, Vec<String>>;

impl SessionManager {
    /// Build a parent → children mapping from the `children` table,
    /// limited to sessions in `self.sessions`.
    async fn build_stop_tree(&self) -> SessionTree {
        let children = self.children.read().await;
        let sessions = self.sessions.read().await;

        // Collect all child IDs that have a parent in the tree.
        let mut all_child_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for child_infos in children.iter().map(|(_, v)| v) {
            for info in child_infos {
                if sessions.contains_key(&info.session_id) {
                    all_child_ids.insert(info.session_id.clone());
                }
            }
        }

        let mut tree: SessionTree = HashMap::new();
        for (parent_id, child_infos) in children.iter() {
            let child_ids: Vec<String> = child_infos
                .iter()
                .filter(|info| sessions.contains_key(&info.session_id))
                .map(|info| info.session_id.clone())
                .collect();
            if !child_ids.is_empty() {
                tree.insert(parent_id.clone(), child_ids);
            }
        }

        // Add root sessions (not a child of any other session) to the tree.
        for session_id in sessions.keys() {
            if !all_child_ids.contains(session_id) && !tree.contains_key(session_id) {
                tree.insert(session_id.clone(), Vec::new());
            }
        }

        tree
    }
}

// ── BFS leaf-to-root ordering ───────────────────────────────────────────

/// Compute stop order from a session tree.
/// Returns levels from leaves (deepest) to roots (shallowest).
/// BFS from roots, then reverse.
pub(crate) fn stop_order_from_tree(tree: &SessionTree) -> Vec<Vec<String>> {
    if tree.is_empty() {
        return Vec::new();
    }

    // Collect all session IDs referenced in the tree.
    let mut all_ids: Vec<String> = tree.keys().cloned().collect();
    for child_ids in tree.values() {
        all_ids.extend(child_ids.iter().cloned());
    }
    all_ids.sort();
    all_ids.dedup();

    // Identify root nodes — IDs that do NOT appear as any node's child.
    let child_set: std::collections::HashSet<&String> = tree.values().flatten().collect();
    let mut roots: Vec<String> = all_ids
        .iter()
        .filter(|id| !child_set.contains(id))
        .cloned()
        .collect();

    if roots.is_empty() {
        // All nodes are in the tree — pick any as root for BFS.
        if let Some(first) = all_ids.first() {
            roots.push(first.clone());
        } else {
            return Vec::new();
        }
    }

    // BFS from roots → levels in root-first order.
    let mut levels: Vec<Vec<String>> = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut queue: VecDeque<String> = roots.into_iter().collect();

    while !queue.is_empty() {
        let level: Vec<String> = queue.drain(..).collect();
        let mut next_level = Vec::new();

        for id in &level {
            if !visited.insert(id.clone()) {
                continue;
            }
            if let Some(children) = tree.get(id) {
                for child in children {
                    if !visited.contains(child) {
                        next_level.push(child.clone());
                    }
                }
            }
        }

        next_level.sort();
        next_level.dedup();
        levels.push(level);
        queue.extend(next_level);
    }

    // Reverse: leaves first, roots last.
    levels.reverse();
    levels
}

// ── per-session stop ────────────────────────────────────────────────────

/// Errors during single-session stop.
#[derive(Debug)]
pub(crate) enum StopError {
    /// Session or ConversationSession not found — skipped.
    Skipped,
    /// Stop or persist failed.
    Failed,
}

impl SessionManager {
    /// Stop a single session and persist its checkpoint.
    ///
    /// Graceful mode waits with no hard timeout; if forceful
    /// escalation is detected during the wait, the session is
    /// force-stopped immediately (per design doc).
    /// Forceful mode stops immediately.
    pub(crate) async fn stop_single_session(
        &self,
        session_id: &str,
        mode: ShutdownMode,
        cascade: bool,
        timeout: Duration,
        progress_tx: Option<tokio::sync::mpsc::Sender<GracefulStopProgress>>,
    ) -> Result<GracefulStopOutcome, StopError> {
        let cs = self
            .get_conversation_session(session_id)
            .await
            .ok_or(StopError::Skipped)?;

        // Forceful path: snapshot → force_kill → persist checkpoint.
        // `stop(cascade)` is NOT called here — it would re-run forced
        // operations (token cancel, cascade, kill) redundantly.
        if mode == ShutdownMode::Forceful {
            // Snapshot transcript state before force-kill clears exec state.
            cs.write().await.snapshot_current_state(
                closeclaw_session::run_health::TranscriptOp::Rewrite,
                "user-stop",
            );
            cs.write().await.force_kill().await;

            // Notify parent about forced termination of run-mode child.
            self.notify_child_forced_termination(session_id).await;

            let pending_ops = {
                let guard = cs.read().await;
                guard.collect_pending_operations()
            };
            return match self
                .persist_checkpoint_with_pending(session_id, pending_ops)
                .await
            {
                Ok(()) => Ok(GracefulStopOutcome::Completed),
                Err(_) => Err(StopError::Failed),
            };
        }

        // Graceful path:
        // 1. Set stopped flag BEFORE cascade and waiting, so
        //    Gateway's is_stopped() check rejects new messages.
        // 2. Snapshot transcript state before stop clears exec state.
        // 3. Call stop(cascade, mode) — handles cascade to children
        //    (recursive graceful stop, waiting for children's
        //    in-flight ops) + cleanup (clear_exec_state).
        // 4. graceful_wait — wait for THIS session's in-flight ops
        //    (LLM stream + tool calls). This happens AFTER cascade
        //    so children's in-flight ops are handled first.
        // 5. Persist checkpoint.
        {
            let guard = cs.read().await;
            let _ = guard.stopped.compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            );
        }
        cs.write().await.snapshot_current_state(
            closeclaw_session::run_health::TranscriptOp::Rewrite,
            "user-stop",
        );

        // Cascade: stop children recursively with Graceful mode.
        // stop() calls cascade_stop_children which recursively calls
        // child.stop(true, Graceful) — each child's in-flight ops are
        // waited on via graceful_stop() before the next sibling is
        // processed. Returns info about any children that timed out.
        let cascade_info = cs.read().await.stop(cascade, mode).await;

        // Wait for THIS session's in-flight ops (after cascade
        // completes, so children's in-flight ops have been handled).
        let (pending_ops, outcome) = self
            .graceful_wait(&cs, session_id, timeout, progress_tx)
            .await;

        match outcome {
            GracefulStopOutcome::Interrupted => {
                tracing::info!(
                    session_id = %session_id,
                    "forceful escalation detected during graceful wait, \
                     force-stopping"
                );
                return self
                    .forceful_stop_session(cs, session_id, cascade)
                    .await
                    .map(|_| GracefulStopOutcome::Interrupted);
            }
            GracefulStopOutcome::TimedOut {
                waiting_items,
                remaining,
            } => {
                // Merge cascade timeout info (children that timed out
                // during the recursive cascade stop) into the waiting
                // items so the caller can report the full picture.
                let mut all_items = waiting_items;
                let cascade_timeout_count = cascade_info.timed_out_children.len();
                for (child_id, elapsed) in cascade_info.timed_out_children {
                    all_items.push((child_id, elapsed));
                }
                let remaining = remaining + cascade_timeout_count;
                tracing::info!(
                    session_id = %session_id,
                    remaining,
                    "graceful stop timed out — returning progress to caller"
                );
                // Do NOT call stop() or persist — caller decides
                // whether to force, continue waiting, or abandon.
                return Ok(GracefulStopOutcome::TimedOut {
                    waiting_items: all_items,
                    remaining,
                });
            }
            GracefulStopOutcome::Completed => {
                // Persist checkpoint below.
            }
        }

        // Clear exec state now that in-flight ops are done.
        // This was previously in stop()'s Graceful branch but was
        // moved here so that graceful_wait() can observe in-flight
        // state on timeout/escalation.
        cs.read().await.clear_exec_state();

        // Persist checkpoint.
        if let Err(e) = self
            .persist_checkpoint_with_pending(session_id, pending_ops)
            .await
        {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "stop_single_session: checkpoint persist failed"
            );
            return Err(StopError::Failed);
        }
        if let Some(tm) = self.get_task_manager().await {
            tm.cleanup_finished().await;
        }
        Ok(GracefulStopOutcome::Completed)
    }

    /// Forcefully stop a session: snapshot → force_kill → persist.
    /// Does NOT call stop() (operations are already done).
    async fn forceful_stop_session(
        &self,
        cs: std::sync::Arc<
            tokio::sync::RwLock<closeclaw_session::llm_session::ConversationSession>,
        >,
        session_id: &str,
        _cascade: bool,
    ) -> Result<GracefulStopOutcome, StopError> {
        // Snapshot transcript state before force-kill.
        cs.write().await.snapshot_current_state(
            closeclaw_session::run_health::TranscriptOp::Rewrite,
            "user-stop",
        );
        cs.write().await.force_kill().await;

        // Notify parent about forced termination of run-mode child.
        self.notify_child_forced_termination(session_id).await;

        let pending_ops = {
            let guard = cs.read().await;
            guard.collect_pending_operations()
        };
        match self
            .persist_checkpoint_with_pending(session_id, pending_ops)
            .await
        {
            Ok(()) => Ok(GracefulStopOutcome::Completed),
            Err(_) => Err(StopError::Failed),
        }
    }

    /// Resolve the effective shutdown mode, checking for escalation.
    async fn resolve_effective_mode(&self, mode: ShutdownMode) -> ShutdownMode {
        match self.get_shutdown_handle().await {
            Some(sh) if sh.is_forceful() => {
                if mode != ShutdownMode::Forceful {
                    tracing::info!(
                        original_mode = ?mode,
                        "escalation detected: switching to forceful mode"
                    );
                }
                ShutdownMode::Forceful
            }
            _ => mode,
        }
    }

    /// Remove a session from all active-tracking tables.
    pub(crate) async fn remove_session(&self, session_id: &str) {
        self.sessions.write().await.remove(session_id);
        self.conversation_sessions.write().await.remove(session_id);
        self.channel_active_sessions
            .write()
            .await
            .retain(|_, sid| sid != session_id);
    }
}
