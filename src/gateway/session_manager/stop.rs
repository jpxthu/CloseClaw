//! Leaf-to-root session shutdown for `SessionManager`.
//!
//! Implements the hierarchical stop order required by the phase-based
//! daemon shutdown design doc:
//!
//! 1. Build a parent → children tree from the `children` tracking table.
//! 2. BFS from roots to leaves, then reverse — stops leaves first,
//!    parents last.  Same-level sessions stop concurrently.
//! 3. Per-session behaviour depends on [`ShutdownMode`]:
//!    - **Graceful**: wait for LLM streaming / tool execution to finish,
//!      then persist checkpoint.  No hard timeout — the user decides
//!      when to escalate to forceful mode.
//!    - **Forceful**: stop immediately, persist checkpoint.

use std::collections::{HashMap, VecDeque};

use crate::daemon::shutdown::ShutdownMode;
use crate::llm::session_state::LlmState;
use crate::session::persistence::{
    PendingOperation, PersistenceError, SessionCheckpoint, SessionStatus,
};

use super::SessionManager;

/// Aggregated result of stopping all sessions.
#[derive(Debug, Default)]
pub struct StopResult {
    /// Sessions stopped successfully.
    pub succeeded: usize,
    /// Sessions where stop or persist failed.
    pub failed: usize,
    /// Sessions skipped (not found or no ConversationSession).
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
    ///
    /// Traversal: BFS from roots to leaves → reverse → stops deepest
    /// nodes first.  Sibling sessions at the same level are stopped
    /// concurrently via [`tokio::join!`].
    ///
    /// Returns a [`StopResult`] with succeeded / failed / skipped counts.
    pub async fn stop_all_sessions(&self, mode: ShutdownMode) -> StopResult {
        let tree = self.build_stop_tree().await;
        let stop_order = stop_order_from_tree(&tree);

        if stop_order.is_empty() {
            tracing::info!("stop_all_sessions: no active sessions");
            return StopResult::default();
        }

        tracing::info!(
            count = stop_order.len(),
            mode = ?mode,
            "stop_all_sessions: starting leaf-to-root shutdown"
        );

        let mut result = StopResult::default();

        for level in &stop_order {
            let futures: Vec<_> = level
                .iter()
                .map(|sid| self.stop_single_session(sid, mode))
                .collect();

            let outcomes = futures::future::join_all(futures).await;
            for outcome in outcomes {
                match outcome {
                    Ok(()) => result.succeeded += 1,
                    Err(StopError::Skipped) => result.skipped += 1,
                    Err(StopError::Failed) => result.failed += 1,
                }
            }
        }

        tracing::info!(
            succeeded = result.succeeded,
            failed = result.failed,
            skipped = result.skipped,
            "stop_all_sessions: complete"
        );

        result
    }
}

// ── tree construction ───────────────────────────────────────────────────

/// Internal session tree built from the `children` tracking table.
/// Maps parent_session_id → list of child session_ids.
type SessionTree = HashMap<String, Vec<String>>;

impl SessionManager {
    /// Build a parent → children mapping from the `children` table,
    /// limited to sessions that still exist in `self.sessions`.
    /// Sessions not present in the `children` table as parents are
    /// treated as root nodes with no children.
    async fn build_stop_tree(&self) -> SessionTree {
        let children = self.children.read().await;
        let sessions = self.sessions.read().await;

        // Collect all child IDs that have a parent in the tree.
        let mut all_child_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for child_infos in children.values() {
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
///
/// Returns levels from leaves (deepest) to roots (shallowest).
/// Each inner `Vec` is a level of sibling sessions that may be
/// stopped concurrently.
///
/// Algorithm:
/// 1. BFS from roots → levels in root-first order.
/// 2. Reverse → levels in leaf-first order.
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

impl SessionManager {
    /// Stop a single session and persist its checkpoint.
    ///
    /// Behaviour depends on `mode`:
    /// - Graceful: wait for LLM streaming and tool execution to
    ///   finish naturally (no hard timeout — the user decides when
    ///   to escalate to forceful mode).
    /// - Forceful: stop immediately regardless of state.
    async fn stop_single_session(
        &self,
        session_id: &str,
        mode: ShutdownMode,
    ) -> Result<(), StopError> {
        let cs = self
            .get_conversation_session(session_id)
            .await
            .ok_or(StopError::Skipped)?;

        // Collect pending operations. Forceful mode collects from
        // tool_states/child_states; graceful mode collects from the
        // assistant message after the streaming loop below.
        let mut pending_ops: Vec<PendingOperation> = if mode == ShutdownMode::Forceful {
            let guard = cs.read().await;
            guard.collect_pending_operations()
        } else {
            Vec::new()
        };

        // Graceful: state-aware loop that distinguishes three sub-states:
        // 1. LLM streaming in progress → wait for stream to end.
        // 2. Stream just ended → extract pending tool calls (if any);
        //    do NOT wait for tools to execute.
        // 3. Tools running (after stream ended) → wait for them to finish.
        // No hard timeout — the user decides when to escalate to forceful.
        if mode == ShutdownMode::Graceful {
            let mut streaming_seen = false;

            loop {
                let (is_streaming, has_running_tools) = {
                    let guard = cs.read().await;
                    let state = *guard.llm_state.read().expect("llm_state lock poisoned");
                    let tool_states = guard.tool_states.read().expect("tool_states lock poisoned");
                    let streaming = matches!(state, LlmState::Receiving | LlmState::Requesting);
                    let tools = tool_states.values().any(|s| {
                        matches!(
                            s,
                            crate::llm::session_state::ToolExecState::RunningForeground
                                | crate::llm::session_state::ToolExecState::RunningBackground
                        )
                    });
                    (streaming, tools)
                };

                if is_streaming {
                    streaming_seen = true;
                } else if streaming_seen {
                    // Stream just ended — extract pending tool calls
                    // from the last assistant message.
                    let ops = {
                        let guard = cs.read().await;
                        guard.extract_pending_tool_calls()
                    };
                    if !ops.is_empty() {
                        pending_ops = ops;
                        break;
                    }
                    // No tool calls requested — if tools are still
                    // running, keep waiting; otherwise we are done.
                    if !has_running_tools {
                        break;
                    }
                    // Tools running but not from assistant tool_use;
                    // fall through to next iteration.
                } else {
                    if has_running_tools {
                        continue; // 工具执行中，等待完成
                    }
                    break; // 真正的 Idle
                }

                tracing::debug!(
                    session_id = %session_id,
                    streaming = is_streaming,
                    running_tools = has_running_tools,
                    "graceful stop: session still active, waiting for completion"
                );
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }

        // Stop the session (cancel token, kill tool handles, clear state).
        cs.read().await.stop(false).await;

        // Persist checkpoint.
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

        // Remove from active sessions.
        self.remove_session(session_id).await;

        Ok(())
    }

    /// Persist a session checkpoint with optional pending operations.
    ///
    /// When `pending_ops` is non-empty (forceful shutdown), the operations
    /// are recorded in the checkpoint so the recovery service can inject
    /// failure results on restart.
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

        // Sync system_appends from ConversationSession.
        {
            let conv = self.conversation_sessions.read().await;
            if let Some(cs) = conv.get(session_id) {
                let guard = cs.read().await;
                cp.system_appends = guard.system_appends().to_vec();
            }
        }

        // Record pending operations from forceful shutdown.
        if !pending_ops.is_empty() {
            cp.pending_operations = pending_ops;
        }

        storage.save_checkpoint(&cp).await
    }

    /// Remove a session from all active-tracking tables.
    async fn remove_session(&self, session_id: &str) {
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
    use crate::gateway::session_manager::spawn::SpawnMode;
    use crate::gateway::session_manager::test_helpers::{
        register_child_only, setup_parent_with_conv,
    };
    use crate::session::bootstrap::BootstrapMode;
    use std::sync::Arc;

    fn make_test_session_manager() -> SessionManager {
        let config = crate::gateway::GatewayConfig::default();
        SessionManager::new(&config, None, None, BootstrapMode::Full, Default::default())
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
        };
        assert_eq!(r.total(), 6);
    }

    #[test]
    fn test_stop_result_default() {
        let r = StopResult::default();
        assert_eq!(r.total(), 0);
    }

    // ── stop_all_sessions integration tests ──────────────────────────────

    #[tokio::test]
    async fn test_stop_all_sessions_empty() {
        let mgr = make_test_session_manager();
        let result = mgr.stop_all_sessions(ShutdownMode::Graceful).await;
        assert_eq!(result.total(), 0);
    }

    #[tokio::test]
    async fn test_stop_all_sessions_idle_session() {
        let mgr = make_test_session_manager();
        let parent_id = "parent-1";
        setup_parent_with_conv(&mgr, parent_id).await;
        let child_id = "child-1";
        register_child_only(&mgr, parent_id, child_id, "worker", SpawnMode::Session).await;
        // Create a ConversationSession for the child
        let cs = std::sync::Arc::new(tokio::sync::RwLock::new(
            crate::llm::session::ConversationSession::new(
                child_id.to_string(),
                "test-model".to_string(),
                std::path::PathBuf::from("/tmp"),
            ),
        ));
        mgr.conversation_sessions
            .write()
            .await
            .insert(child_id.to_string(), cs);
        // Add child to sessions map (build_stop_tree filters on this)
        mgr.sessions.write().await.insert(
            child_id.to_string(),
            crate::gateway::Session {
                id: child_id.to_string(),
                agent_id: "worker".to_string(),
                channel: "feishu".to_string(),
                created_at: chrono::Utc::now().timestamp(),
                depth: 1,
            },
        );

        let result = mgr.stop_all_sessions(ShutdownMode::Graceful).await;
        // Both parent and child have ConversationSessions → both stopped.
        assert!(result.succeeded >= 1);
    }

    #[tokio::test]
    async fn test_stop_all_sessions_forceful() {
        let mgr = make_test_session_manager();
        let parent_id = "parent-2";
        setup_parent_with_conv(&mgr, parent_id).await;

        let result = mgr.stop_all_sessions(ShutdownMode::Forceful).await;
        // Parent has no storage → persist_checkpoint returns Ok (no-op).
        // Parent should be stopped.
        assert!(result.succeeded >= 1);
    }

    // ── multi-layer tree ordering ─────────────────────────────────────

    #[test]
    fn test_stop_order_three_level_tree() {
        // root → [child1, child2], child1 → [grandchild_a, grandchild_b],
        // grandchild_a → great_grandchild
        let mut tree = HashMap::new();
        tree.insert(
            "root".to_string(),
            vec!["child1".to_string(), "child2".to_string()],
        );
        tree.insert(
            "child1".to_string(),
            vec!["grandchild_a".to_string(), "grandchild_b".to_string()],
        );
        tree.insert(
            "grandchild_a".to_string(),
            vec!["great_grandchild".to_string()],
        );

        let order = stop_order_from_tree(&tree);
        // 4 levels: [[great_grandchild], [grandchild_a, grandchild_b],
        //            [child1, child2], [root]]
        assert_eq!(order.len(), 4);
        assert_eq!(order[0], vec!["great_grandchild"]);
        assert_eq!(order[1].len(), 2);
        assert!(order[1].contains(&"grandchild_a".to_string()));
        assert!(order[1].contains(&"grandchild_b".to_string()));
        assert_eq!(order[2].len(), 2);
        assert!(order[2].contains(&"child1".to_string()));
        assert!(order[2].contains(&"child2".to_string()));
        assert_eq!(order[3], vec!["root"]);
    }

    #[test]
    fn test_stop_order_multiple_roots() {
        // Two independent trees: root1 → child1, root2 → child2
        let mut tree = HashMap::new();
        tree.insert("root1".to_string(), vec!["child1".to_string()]);
        tree.insert("root2".to_string(), vec!["child2".to_string()]);

        let order = stop_order_from_tree(&tree);
        // Leaves first: [[child1, child2], [root1, root2]]
        assert_eq!(order.len(), 2);
        assert_eq!(order[0].len(), 2);
        assert!(order[0].contains(&"child1".to_string()));
        assert!(order[0].contains(&"child2".to_string()));
        assert_eq!(order[1].len(), 2);
        assert!(order[1].contains(&"root1".to_string()));
        assert!(order[1].contains(&"root2".to_string()));
    }

    // ── per-state behavior tests ──────────────────────────────────────

    #[tokio::test]
    async fn test_stop_sessions_forceful_skips_streaming_wait() {
        use crate::gateway::session_manager::test_helpers::register_child_only;
        use crate::llm::session_state::LlmState;

        let mgr = make_test_session_manager();
        let parent_id = "parent-streaming";
        setup_parent_with_conv(&mgr, parent_id).await;
        let child_id = "child-streaming";
        register_child_only(&mgr, parent_id, child_id, "worker", SpawnMode::Session).await;

        // Create a ConversationSession for the child
        let cs = Arc::new(tokio::sync::RwLock::new(
            crate::llm::session::ConversationSession::new(
                child_id.to_string(),
                "test-model".to_string(),
                std::path::PathBuf::from("/tmp"),
            ),
        ));
        // Set LLM state to Receiving (streaming)
        {
            let guard = cs.read().await;
            let mut state = guard.llm_state.write().expect("llm_state lock poisoned");
            *state = LlmState::Receiving;
        }
        mgr.conversation_sessions
            .write()
            .await
            .insert(child_id.to_string(), cs.clone());
        mgr.sessions.write().await.insert(
            child_id.to_string(),
            crate::gateway::Session {
                id: child_id.to_string(),
                agent_id: "worker".to_string(),
                channel: "feishu".to_string(),
                created_at: chrono::Utc::now().timestamp(),
                depth: 1,
            },
        );

        // Forceful mode should not wait for streaming to finish
        let start = tokio::time::Instant::now();
        let result = mgr.stop_all_sessions(ShutdownMode::Forceful).await;
        let elapsed = start.elapsed();

        // Should complete quickly (not waiting for the full graceful timeout)
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "forceful mode should not wait for streaming, took {:?}",
            elapsed
        );
        assert!(result.succeeded >= 1);
    }

    #[tokio::test]
    async fn test_stop_sessions_forceful_with_tool_running() {
        use crate::gateway::session_manager::test_helpers::register_child_only;
        use crate::llm::session_state::ToolExecState;

        let mgr = make_test_session_manager();
        let parent_id = "parent-tool";
        setup_parent_with_conv(&mgr, parent_id).await;
        let child_id = "child-tool";
        register_child_only(&mgr, parent_id, child_id, "worker", SpawnMode::Session).await;

        let cs = Arc::new(tokio::sync::RwLock::new(
            crate::llm::session::ConversationSession::new(
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
            crate::gateway::Session {
                id: child_id.to_string(),
                agent_id: "worker".to_string(),
                channel: "feishu".to_string(),
                created_at: chrono::Utc::now().timestamp(),
                depth: 1,
            },
        );

        // Forceful mode should stop without waiting
        let result = mgr.stop_all_sessions(ShutdownMode::Forceful).await;
        assert!(result.succeeded >= 1);
    }

    #[tokio::test]
    async fn test_stop_sessions_forceful_empty_sessions_map() {
        let mgr = make_test_session_manager();
        // No sessions registered at all
        let result = mgr.stop_all_sessions(ShutdownMode::Forceful).await;
        assert_eq!(result.total(), 0);
    }

    #[tokio::test]
    async fn test_stop_result_total_various() {
        let r = StopResult {
            succeeded: 0,
            failed: 0,
            skipped: 0,
        };
        assert_eq!(r.total(), 0);

        let r = StopResult {
            succeeded: usize::MAX,
            failed: 0,
            skipped: 0,
        };
        assert_eq!(r.total(), usize::MAX);
    }
}
