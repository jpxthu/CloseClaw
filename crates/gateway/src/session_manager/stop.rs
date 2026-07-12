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
use std::sync::Arc;
use std::time::Duration;

use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_llm::session_state::LlmState;
use closeclaw_session::persistence::{
    PendingOperation, PersistenceError, SessionCheckpoint, SessionStatus,
};

use super::SessionManager;

/// Default graceful shutdown timeout.
pub const DEFAULT_GRACEFUL_TIMEOUT: Duration = Duration::from_secs(30);

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
    /// Sessions where graceful stop timed out (session not killed).
    pub graceful_timeouts: Vec<GracefulTimeoutInfo>,
}

/// Information about a session whose graceful stop timed out.
#[derive(Debug, Clone)]
pub struct GracefulTimeoutInfo {
    pub session_id: String,
    pub waiting_items: Vec<String>,
    pub elapsed: Duration,
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
        progress_tx: Option<&tokio::sync::mpsc::Sender<StopProgress>>,
        graceful_timeout: Duration,
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
            // Dynamically query the shutdown handle for the current mode.
            // After escalation (e.g. graceful → forceful), the next level
            // will use the escalated mode instead of the original `mode`.
            let effective_mode = match self.get_shutdown_handle().await {
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
            };

            let (count, timeouts) = self
                .process_stop_level(
                    level,
                    effective_mode,
                    &mut result,
                    progress_tx,
                    total_sessions.saturating_sub(processed),
                    graceful_timeout,
                )
                .await;
            processed += count;
            result.graceful_timeouts.extend(timeouts);
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
    /// Returns (count, timeout infos).
    async fn process_stop_level(
        &self,
        level: &[String],
        mode: ShutdownMode,
        result: &mut StopResult,
        progress_tx: Option<&tokio::sync::mpsc::Sender<StopProgress>>,
        remaining: usize,
        graceful_timeout: Duration,
    ) -> (usize, Vec<GracefulTimeoutInfo>) {
        let futures: Vec<_> = level
            .iter()
            .map(|sid| self.stop_single_session(sid, mode, graceful_timeout))
            .collect();

        let outcomes = futures::future::join_all(futures).await;
        let count = level.len();
        let mut timeouts = Vec::new();
        for (idx, outcome) in outcomes.into_iter().enumerate() {
            let (success, timeout_info) = Self::process_stop_outcome(outcome, result);
            if let Some(info) = timeout_info {
                timeouts.push(info);
            }
            Self::notify_stop_progress(
                progress_tx,
                &level[idx],
                success,
                remaining.saturating_sub(idx + 1),
            )
            .await;
        }
        (count, timeouts)
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

    /// Record a stop outcome and return (success, optional timeout info).
    fn process_stop_outcome(
        outcome: Result<StopSingleResult, StopError>,
        result: &mut StopResult,
    ) -> (bool, Option<GracefulTimeoutInfo>) {
        match outcome {
            Ok(r) => {
                result.succeeded += 1;
                (true, r.graceful_timeout)
            }
            Err(StopError::Skipped) => {
                result.skipped += 1;
                (false, None)
            }
            Err(StopError::Failed) => {
                result.failed += 1;
                (false, None)
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
fn stop_order_from_tree(tree: &SessionTree) -> Vec<Vec<String>> {
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
enum StopError {
    /// Session or ConversationSession not found — skipped.
    Skipped,
    /// Stop or persist failed.
    Failed,
}

/// Result of stopping a single session.
struct StopSingleResult {
    /// Whether the stop completed successfully (all operations finished).
    /// True also when a graceful timeout occurred but the session was
    /// not killed — the caller can inspect `graceful_timeout` for details.
    _completed: bool,
    /// If the graceful stop timed out, contains waiting item info.
    graceful_timeout: Option<GracefulTimeoutInfo>,
}

impl SessionManager {
    /// Stop a single session and persist its checkpoint.
    ///
    /// Graceful mode waits with a configurable timeout; on timeout,
    /// the session is NOT killed — waiting item info is returned.
    /// Forceful mode stops immediately.
    async fn stop_single_session(
        &self,
        session_id: &str,
        mode: ShutdownMode,
        graceful_timeout: Duration,
    ) -> Result<StopSingleResult, StopError> {
        let cs = self
            .get_conversation_session(session_id)
            .await
            .ok_or(StopError::Skipped)?;

        // Forceful: collect pending ops from live state, skip wait.
        if mode == ShutdownMode::Forceful {
            let pending_ops = {
                let guard = cs.read().await;
                guard.collect_pending_operations()
            };
            return self
                .finalize_session_stop(cs, session_id, pending_ops)
                .await;
        }

        // Graceful: state-aware loop with configurable timeout.
        let (pending_ops, timeout_info) =
            Self::graceful_wait_with_timeout(&cs, session_id, graceful_timeout).await;

        if let Some(info) = timeout_info {
            tracing::warn!(
                session_id = %session_id,
                elapsed = ?info.elapsed,
                waiting = ?info.waiting_items,
                "graceful stop timed out: session not killed"
            );
            return Ok(StopSingleResult {
                _completed: false,
                graceful_timeout: Some(info),
            });
        }

        self.finalize_session_stop(cs, session_id, pending_ops)
            .await
    }

    /// Graceful wait with configurable timeout.
    /// Returns pending ops and, on timeout, [`GracefulTimeoutInfo`].
    async fn graceful_wait_with_timeout(
        cs: &Arc<tokio::sync::RwLock<closeclaw_session::llm_session::ConversationSession>>,
        session_id: &str,
        timeout: Duration,
    ) -> (Vec<PendingOperation>, Option<GracefulTimeoutInfo>) {
        let start = tokio::time::Instant::now();
        let mut pending_ops = Vec::new();
        let mut streaming_seen = false;

        let result = tokio::time::timeout(timeout, async {
            loop {
                let (is_streaming, has_running_tools) = {
                    let guard: &closeclaw_session::llm_session::ConversationSession =
                        &*cs.read().await;
                    let state = *guard.llm_state.read().expect("llm_state lock poisoned");
                    let tool_states = guard.tool_states.read().expect("tool_states lock poisoned");
                    let streaming = matches!(state, LlmState::Receiving | LlmState::Requesting);
                    let tools = tool_states.values().any(|s| {
                        matches!(
                            s,
                            closeclaw_llm::session_state::ToolExecState::RunningForeground
                                | closeclaw_llm::session_state::ToolExecState::RunningBackground
                        )
                    });
                    (streaming, tools)
                };

                if is_streaming {
                    streaming_seen = true;
                } else if streaming_seen {
                    let ops = {
                        let guard: &closeclaw_session::llm_session::ConversationSession =
                            &*cs.read().await;
                        guard.extract_pending_tool_calls()
                    };
                    if !ops.is_empty() {
                        pending_ops = ops;
                        break;
                    }
                    if !has_running_tools {
                        break;
                    }
                } else if has_running_tools {
                    continue;
                } else {
                    break;
                }

                tracing::debug!(
                    session_id = %session_id,
                    streaming = is_streaming,
                    running_tools = has_running_tools,
                    "graceful stop: waiting for completion"
                );
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await;

        match result {
            Ok(()) => (pending_ops, None),
            Err(_elapsed) => {
                let waiting_items = Self::collect_waiting_items(cs).await;
                (
                    pending_ops,
                    Some(GracefulTimeoutInfo {
                        session_id: session_id.to_string(),
                        waiting_items,
                        elapsed: start.elapsed(),
                    }),
                )
            }
        }
    }

    /// Operations still in progress (for timeout reporting).
    async fn collect_waiting_items(
        cs: &Arc<tokio::sync::RwLock<closeclaw_session::llm_session::ConversationSession>>,
    ) -> Vec<String> {
        let guard = cs.read().await;
        let mut items = Vec::new();
        let state = *guard.llm_state.read().expect("lock poisoned");
        if matches!(state, LlmState::Receiving | LlmState::Requesting) {
            items.push("LLM streaming".to_string());
        }
        for (id, s) in guard.tool_states.read().expect("lock poisoned").iter() {
            if matches!(
                s,
                closeclaw_llm::session_state::ToolExecState::RunningForeground
                    | closeclaw_llm::session_state::ToolExecState::RunningBackground
            ) {
                items.push(format!("tool {} running", id));
            }
        }
        for (id, s) in guard.child_states.read().expect("lock poisoned").iter() {
            if matches!(s, closeclaw_common::ChildSessionState::Running) {
                items.push(format!("child session {} running", id));
            }
        }
        items
    }

    /// Finalize a stopped session: stop, clean up, persist.
    async fn finalize_session_stop(
        &self,
        cs: Arc<tokio::sync::RwLock<closeclaw_session::llm_session::ConversationSession>>,
        session_id: &str,
        pending_ops: Vec<PendingOperation>,
    ) -> Result<StopSingleResult, StopError> {
        cs.read().await.stop(false).await;
        if let Some(tm) = self.get_task_manager().await {
            tm.cleanup_finished().await;
        }
        if let Err(e) = self
            .persist_checkpoint_with_pending(session_id, pending_ops)
            .await
        {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "stop_all_sessions: checkpoint persist failed"
            );
            return Err(StopError::Failed);
        }

        Ok(StopSingleResult {
            _completed: true,
            graceful_timeout: None,
        })
    }

    /// Persist a session checkpoint with optional pending operations.
    /// Non-empty `pending_ops` (forceful shutdown) are recorded for recovery.
    async fn persist_checkpoint_with_pending(
        &self,
        session_id: &str,
        pending_ops: Vec<PendingOperation>,
    ) -> Result<(), PersistenceError> {
        let storage = {
            let guard = self.storage.read().await;
            match guard.as_ref() {
                Some(s) => std::sync::Arc::clone(s),
                None => return Ok(()),
            }
        };

        let (agent_id, channel) = {
            let sessions = self.sessions.read().await;
            match sessions.get(session_id) {
                Some(s) => (Some(s.agent_id.clone()), Some(s.channel.clone())),
                None => (None, None),
            }
        };

        let pending = {
            let conv = self.conversation_sessions.read().await;
            match conv.get(session_id) {
                Some(cs) => {
                    let guard = cs.read().await;
                    guard.get_pending_messages()
                }
                None => Vec::new(),
            }
        };

        let mut cp = match storage.load_checkpoint(session_id).await? {
            Some(mut cp) => {
                cp.status = SessionStatus::Active;
                if let Some(ch) = channel {
                    cp.platform = Some(ch);
                }
                if let Some(aid) = agent_id {
                    cp.agent_id = Some(aid);
                }
                cp.pending_messages = pending;
                cp
            }
            None => {
                let mut cp = SessionCheckpoint::new(session_id.to_string())
                    .with_status(SessionStatus::Active);
                if let Some(ch) = channel {
                    cp = cp.with_platform(ch);
                }
                if let Some(aid) = agent_id {
                    cp = cp.with_agent_id(aid);
                }
                cp.with_pending_messages(pending)
            }
        };

        // Sync system_appends and verbosity_level from ConversationSession.
        {
            let conv = self.conversation_sessions.read().await;
            if let Some(cs) = conv.get(session_id) {
                let guard = cs.read().await;
                cp.system_appends = guard.user_system_appends().to_vec();
                cp.verbosity_level = guard.verbosity_level();
            }
        }

        // Record pending operations from forceful shutdown.
        if !pending_ops.is_empty() {
            cp.pending_operations = pending_ops;
        }

        storage.save_checkpoint(&cp).await
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

// ── tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_manager::spawn::SpawnMode;
    use crate::session_manager::test_helpers::setup_parent_with_conv;
    use std::sync::Arc;

    fn make_test_session_manager() -> SessionManager {
        let config = crate::GatewayConfig::default();
        SessionManager::new(&config, None, None, Default::default())
    }

    // ── stop_order_from_tree tests ───────────────────────────────────────

    #[test]
    fn test_stop_order_empty_tree() {
        let tree = HashMap::new();
        let order = stop_order_from_tree(&tree);
        assert!(order.is_empty());
    }

    #[test]
    fn test_stop_order_linear_chain() {
        // root → child1 → grandchild
        let mut tree = HashMap::new();
        tree.insert("root".to_string(), vec!["child1".to_string()]);
        tree.insert("child1".to_string(), vec!["grandchild".to_string()]);
        let order = stop_order_from_tree(&tree);
        // Leaves to root: [[grandchild], [child1], [root]]
        assert_eq!(order.len(), 3);
        assert_eq!(order[0], vec!["grandchild"]);
        assert_eq!(order[1], vec!["child1"]);
        assert_eq!(order[2], vec!["root"]);
    }

    #[test]
    fn test_stop_order_breadth_first() {
        // root → [child1, child2], child1 → grandchild
        let mut tree = HashMap::new();
        tree.insert(
            "root".to_string(),
            vec!["child1".to_string(), "child2".to_string()],
        );
        tree.insert("child1".to_string(), vec!["grandchild".to_string()]);
        let order = stop_order_from_tree(&tree);
        // Reverse BFS: [[grandchild], [child1, child2], [root]]
        assert_eq!(order.len(), 3);
        assert_eq!(order[0], vec!["grandchild"]);
        assert_eq!(order[1].len(), 2);
        assert!(order[1].contains(&"child1".to_string()));
        assert!(order[1].contains(&"child2".to_string()));
        assert_eq!(order[2], vec!["root"]);
    }

    #[test]
    fn test_stop_order_diamond() {
        // root → [left, right], both → shared_child
        let mut tree = HashMap::new();
        tree.insert(
            "root".to_string(),
            vec!["left".to_string(), "right".to_string()],
        );
        tree.insert("left".to_string(), vec!["shared_child".to_string()]);
        tree.insert("right".to_string(), vec!["shared_child".to_string()]);
        let order = stop_order_from_tree(&tree);
        // shared_child deduped → [[shared_child], [left, right], [root]]
        assert_eq!(order.len(), 3);
        assert_eq!(order[0], vec!["shared_child"]);
        assert_eq!(order[1].len(), 2);
        assert_eq!(order[2], vec!["root"]);
    }

    // ── StopResult tests ─────────────────────────────────────────────────

    #[test]
    fn test_stop_result_total() {
        let r = StopResult {
            succeeded: 3,
            failed: 1,
            skipped: 2,
            ..Default::default()
        };
        assert_eq!(r.total(), 6);
    }

    // ── stop_all_sessions integration tests ──────────────────────────────

    #[tokio::test]
    async fn test_stop_all_sessions_empty() {
        let mgr = make_test_session_manager();
        let result = mgr
            .stop_all_sessions(ShutdownMode::Graceful, None, DEFAULT_GRACEFUL_TIMEOUT)
            .await;
        assert_eq!(result.total(), 0);
    }

    #[tokio::test]
    async fn test_stop_all_sessions_forceful() {
        let mgr = make_test_session_manager();
        let parent_id = "parent-2";
        setup_parent_with_conv(&mgr, parent_id).await;

        let result = mgr
            .stop_all_sessions(ShutdownMode::Forceful, None, DEFAULT_GRACEFUL_TIMEOUT)
            .await;
        // Parent has no storage → persist_checkpoint returns Ok (no-op).
        // Parent should be stopped.
        assert!(result.succeeded >= 1);
    }

    // ── multi-layer tree ordering ─────────────────────────────────────

    #[test]
    fn test_stop_order_multiple_roots() {
        let mut tree = HashMap::new();
        tree.insert("root1".to_string(), vec!["child1".to_string()]);
        tree.insert("root2".to_string(), vec!["child2".to_string()]);
        let order = stop_order_from_tree(&tree);
        assert_eq!(order.len(), 2);
        assert_eq!(order[0].len(), 2);
        assert_eq!(order[1].len(), 2);
    }

    // ── per-state behavior tests ──────────────────────────────────────

    #[tokio::test]
    async fn test_stop_sessions_forceful_with_tool_running() {
        use crate::session_manager::test_helpers::register_child_only;
        use closeclaw_llm::session_state::ToolExecState;

        let mgr = make_test_session_manager();
        let parent_id = "parent-tool";
        setup_parent_with_conv(&mgr, parent_id).await;
        let child_id = "child-tool";
        register_child_only(&mgr, parent_id, child_id, "worker", SpawnMode::Session).await;

        let cs = Arc::new(tokio::sync::RwLock::new(
            closeclaw_session::llm_session::ConversationSession::new(
                child_id.to_string(),
                "test-model".to_string(),
                std::path::PathBuf::from("/tmp"),
            ),
        ));
        // Set a tool to RunningForeground state
        {
            let guard = cs.read().await;
            let mut tool_states = guard
                .tool_states
                .write()
                .expect("tool_states lock poisoned");
            tool_states.insert("tool-1".to_string(), ToolExecState::RunningForeground);
        }
        mgr.conversation_sessions
            .write()
            .await
            .insert(child_id.to_string(), cs.clone());
        mgr.sessions.write().await.insert(
            child_id.to_string(),
            crate::Session {
                id: child_id.to_string(),
                agent_id: "worker".to_string(),
                channel: "feishu".to_string(),
                created_at: chrono::Utc::now().timestamp(),
                depth: 1,
            },
        );

        // Forceful mode should stop without waiting
        let result = mgr
            .stop_all_sessions(ShutdownMode::Forceful, None, DEFAULT_GRACEFUL_TIMEOUT)
            .await;
        assert!(result.succeeded >= 1);
    }

    // ── StopProgress callback tests ──────────────────────────────────

    #[tokio::test]
    async fn test_stop_all_sessions_with_progress_callback() {
        let mgr = make_test_session_manager();
        let parent_id = "parent-prog";
        setup_parent_with_conv(&mgr, parent_id).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel::<StopProgress>(16);

        let result = mgr
            .stop_all_sessions(ShutdownMode::Forceful, Some(&tx), DEFAULT_GRACEFUL_TIMEOUT)
            .await;
        assert!(result.succeeded >= 1);

        // Should have received at least one progress event
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        assert!(!events.is_empty(), "should receive progress events");

        // Each event should have session_id = parent-prog
        assert!(events.iter().any(|e| e.session_id == "parent-prog"));
    }

    #[tokio::test]
    async fn test_stop_progress_remaining_accuracy() {
        let mgr = make_test_session_manager();
        let parent_id = "parent-remaining";
        setup_parent_with_conv(&mgr, parent_id).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel::<StopProgress>(16);

        let result = mgr
            .stop_all_sessions(ShutdownMode::Forceful, Some(&tx), DEFAULT_GRACEFUL_TIMEOUT)
            .await;
        assert_eq!(result.succeeded, 1);

        // Collect all events
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].remaining, 0, "last event should have remaining=0");
        assert!(events[0].success, "parent should stop successfully");
    }

    // ── Step 1.2: graceful timeout tests ──────────────────────────────

    /// Helper: register a child with a ConversationSession.
    async fn setup_child(mgr: &SessionManager, pid: &str, cid: &str) {
        use crate::session_manager::test_helpers::register_child_only;
        register_child_only(mgr, pid, cid, "worker", SpawnMode::Session).await;
        let cs = Arc::new(tokio::sync::RwLock::new(
            closeclaw_session::llm_session::ConversationSession::new(
                cid.to_string(),
                "test-model".to_string(),
                std::path::PathBuf::from("/tmp"),
            ),
        ));
        mgr.conversation_sessions
            .write()
            .await
            .insert(cid.to_string(), cs);
        mgr.sessions.write().await.insert(
            cid.to_string(),
            crate::Session {
                id: cid.to_string(),
                agent_id: "worker".to_string(),
                channel: "feishu".to_string(),
                created_at: chrono::Utc::now().timestamp(),
                depth: 1,
            },
        );
    }

    /// Helper: set LLM state on a session.
    async fn set_llm(mgr: &SessionManager, sid: &str, state: LlmState) {
        let cs = mgr.get_conversation_session(sid).await.unwrap();
        let guard = cs.read().await;
        *guard.llm_state.write().expect("lock") = state;
    }

    /// Helper: set tool state on a session.
    async fn set_tool(
        mgr: &SessionManager,
        sid: &str,
        tid: &str,
        s: closeclaw_llm::session_state::ToolExecState,
    ) {
        let cs = mgr.get_conversation_session(sid).await.unwrap();
        let guard = cs.read().await;
        guard
            .tool_states
            .write()
            .expect("lock")
            .insert(tid.to_string(), s);
    }

    /// Graceful timeout fires while LLM streaming → does not hang.
    #[tokio::test]
    async fn test_graceful_timeout_streaming_returns_info() {
        let mgr = make_test_session_manager();
        let pid = "parent-to-stream";
        setup_parent_with_conv(&mgr, pid).await;
        let cid = "child-to-stream";
        setup_child(&mgr, pid, cid).await;
        set_llm(&mgr, cid, LlmState::Receiving).await;
        let r = mgr
            .stop_all_sessions(
                ShutdownMode::Graceful,
                None,
                std::time::Duration::from_millis(200),
            )
            .await;
        assert!(r.total() >= 1);
    }

    /// Graceful timeout fires while tool running → does not hang.
    #[tokio::test]
    async fn test_graceful_timeout_tool_running_returns_info() {
        let mgr = make_test_session_manager();
        let pid = "parent-to-tool";
        setup_parent_with_conv(&mgr, pid).await;
        let cid = "child-to-tool";
        setup_child(&mgr, pid, cid).await;
        set_tool(
            &mgr,
            cid,
            "tool永不结束",
            closeclaw_llm::session_state::ToolExecState::RunningForeground,
        )
        .await;
        let r = mgr
            .stop_all_sessions(
                ShutdownMode::Graceful,
                None,
                std::time::Duration::from_millis(200),
            )
            .await;
        assert!(r.total() >= 1);
    }

    /// Idle session stops immediately within timeout.
    #[tokio::test]
    async fn test_graceful_timeout_idle_completes_immediately() {
        let mgr = make_test_session_manager();
        setup_parent_with_conv(&mgr, "parent-idle-to").await;
        let r = mgr
            .stop_all_sessions(
                ShutdownMode::Graceful,
                None,
                std::time::Duration::from_millis(200),
            )
            .await;
        assert!(r.succeeded >= 1);
    }

    /// Forceful mode ignores timeout entirely.
    #[tokio::test]
    async fn test_forceful_ignores_timeout() {
        let mgr = make_test_session_manager();
        let pid = "parent-force-to";
        setup_parent_with_conv(&mgr, pid).await;
        let cid = "child-force-to";
        setup_child(&mgr, pid, cid).await;
        set_llm(&mgr, cid, LlmState::Receiving).await;
        let start = tokio::time::Instant::now();
        let r = mgr
            .stop_all_sessions(
                ShutdownMode::Forceful,
                None,
                std::time::Duration::from_millis(50),
            )
            .await;
        assert!(r.succeeded >= 1);
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }
}
