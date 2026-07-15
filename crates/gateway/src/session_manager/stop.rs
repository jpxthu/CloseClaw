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
            let success = Self::process_stop_outcome(outcome, result);
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
        outcome: Result<StopSingleResult, StopError>,
        result: &mut StopResult,
    ) -> bool {
        match outcome {
            Ok(_) => {
                result.succeeded += 1;
                true
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

/// Result of stopping a single session.
pub(crate) struct StopSingleResult {
    /// Whether the stop completed successfully (all operations finished).
    pub(crate) _completed: bool,
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
    ) -> Result<StopSingleResult, StopError> {
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
            let pending_ops = {
                let guard = cs.read().await;
                guard.collect_pending_operations()
            };
            return match self
                .persist_checkpoint_with_pending(session_id, pending_ops)
                .await
            {
                Ok(()) => Ok(StopSingleResult { _completed: true }),
                Err(_) => Err(StopError::Failed),
            };
        }

        // Graceful path:
        // 1. Set stopped flag BEFORE waiting, so Gateway's
        //    is_stopped() check rejects new messages during wait.
        // 2. Snapshot transcript state before stop clears exec state.
        // 3. Call stop(cascade, mode) — handles cascade + cleanup.
        //    stop() internally does NOT call graceful_stop() (that
        //    was the double-wait bug fixed in Step 1.7).
        // 4. Persist checkpoint.
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

        let (pending_ops, interrupted) = self
            .graceful_wait(&cs, session_id, timeout, progress_tx)
            .await;

        if interrupted {
            tracing::info!(
                session_id = %session_id,
                "forceful escalation detected during graceful wait, \
                 force-stopping"
            );
            return self.forceful_stop_session(cs, session_id, cascade).await;
        }

        // stop() handles cascade (to children) + cleanup. The
        // stopped flag is already set, so stop()'s Graceful branch
        // skips the idempotency guard and proceeds with cascade
        // and clear_exec_state.
        cs.read().await.stop(cascade, mode).await;

        // Persist checkpoint (no stop() call inside).
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
        Ok(StopSingleResult { _completed: true })
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
    ) -> Result<StopSingleResult, StopError> {
        // Snapshot transcript state before force-kill.
        cs.write().await.snapshot_current_state(
            closeclaw_session::run_health::TranscriptOp::Rewrite,
            "user-stop",
        );
        cs.write().await.force_kill().await;

        let pending_ops = {
            let guard = cs.read().await;
            guard.collect_pending_operations()
        };
        match self
            .persist_checkpoint_with_pending(session_id, pending_ops)
            .await
        {
            Ok(()) => Ok(StopSingleResult { _completed: true }),
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
