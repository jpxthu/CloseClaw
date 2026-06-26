//! Session recovery service
//!
//! Provides functionality to recover sessions from persisted checkpoints
//! during gateway startup, including spawn_tree reconstruction.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::persistence::{PersistenceError, PersistenceService, SessionCheckpoint};

/// Recovery report containing results of the recovery process
#[derive(Debug)]
pub struct RecoveryReport {
    /// List of session IDs that were successfully recovered
    pub recovered: Vec<String>,
    /// List of session IDs that failed to recover
    pub failed: Vec<String>,
    /// Spawn tree reconstructed from recovered checkpoints
    pub spawn_tree: SpawnTree,
    /// List of session IDs that had pending operations (dirty sessions)
    pub dirty_sessions: Vec<String>,
}

impl RecoveryReport {
    /// Returns true if all sessions were recovered successfully
    pub fn is_full_success(&self) -> bool {
        self.failed.is_empty()
    }

    /// Returns the total number of sessions processed
    pub fn total(&self) -> usize {
        self.recovered.len() + self.failed.len()
    }
}

/// Session recovery service — recovers sessions from persisted checkpoints
pub struct SessionRecoveryService<S: PersistenceService + ?Sized> {
    storage: Arc<S>,
    /// Callback to restore a session from checkpoint
    /// The closure receives the session_id and checkpoint, and should restore the session state.
    #[allow(clippy::type_complexity)]
    restore_fn: RwLock<
        Option<
            Box<
                dyn Fn(
                        &str,
                        &SessionCheckpoint,
                        Option<&str>,
                        &[String],
                    ) -> Result<(), PersistenceError>
                    + Send
                    + Sync,
            >,
        >,
    >,
}

impl<S: PersistenceService + ?Sized> SessionRecoveryService<S> {
    /// Create a new SessionRecoveryService
    pub fn new(storage: Arc<S>) -> Self {
        Self {
            storage,
            restore_fn: RwLock::new(None),
        }
    }

    /// Set the restore callback
    ///
    /// The callback will be invoked for each session during recovery.
    /// It receives the session_id, checkpoint, optional recovery notification
    /// text, and any tool failure result strings to inject.
    pub async fn set_restore_callback<F>(&self, callback: F)
    where
        F: Fn(&str, &SessionCheckpoint, Option<&str>, &[String]) -> Result<(), PersistenceError>
            + Send
            + Sync
            + 'static,
    {
        let mut restore_fn = self.restore_fn.write().await;
        *restore_fn = Some(Box::new(callback));
    }

    /// 执行恢复流程
    ///
    /// 扫描 storage 中所有 active session 并逐一恢复。恢复后根据 checkpoint
    /// 数据重建 spawn_tree：
    /// - 有 `parent_session_id` 且父 session 也已恢复 → 注册为父节点的子节点
    /// - 有 `parent_session_id` 但父 session 未恢复（已被 sweep）→ 降级为根节点，depth 重置为 0
    /// - 无 `parent_session_id` → 确认为根节点
    pub async fn recover(&self) -> Result<RecoveryReport, PersistenceError> {
        let active_sessions = self.storage.list_active_sessions().await?;
        let mut recovered = Vec::new();
        let mut failed = Vec::new();
        let mut checkpoints: HashMap<String, SessionCheckpoint> = HashMap::new();

        for session_id in &active_sessions {
            match self.recover_session(session_id).await {
                Ok(()) => {
                    recovered.push(session_id.clone());
                    // Load checkpoint for spawn_tree reconstruction
                    if let Ok(Some(cp)) = self.storage.load_checkpoint(session_id).await {
                        checkpoints.insert(session_id.clone(), cp);
                    }
                }
                Err(e) => {
                    tracing::error!(
                        session_id = %session_id,
                        "Failed to recover session: {}",
                        e
                    );
                    failed.push(session_id.clone());
                }
            }
        }

        // Collect dirty sessions (those with pending operations)
        let dirty_sessions: Vec<String> = checkpoints
            .iter()
            .filter(|(_, cp)| !cp.pending_operations.is_empty())
            .map(|(id, _)| id.clone())
            .collect();

        // Inject recovery notifications for dirty sessions
        for session_id in &dirty_sessions {
            if let Some(cp) = checkpoints.get_mut(session_id) {
                self.inject_recovery_notifications(session_id, cp);
            }
        }

        // Persist dirty checkpoints with injected notifications
        for session_id in &dirty_sessions {
            if let Some(cp) = checkpoints.get(session_id) {
                if let Err(e) = self.storage.save_checkpoint(cp).await {
                    tracing::error!(
                        session_id = %session_id,
                        "Failed to persist checkpoint with recovery notification: {}",
                        e
                    );
                }
            }
        }

        let (spawn_tree, demoted) = Self::build_spawn_tree(&mut checkpoints, &recovered);

        // 持久化降级后的 checkpoint（depth 重置为 0）
        for session_id in &demoted {
            if let Some(cp) = checkpoints.get(session_id) {
                if let Err(e) = self.storage.save_checkpoint(cp).await {
                    tracing::error!(
                        session_id = %session_id,
                        "Failed to persist demoted checkpoint: {}",
                        e
                    );
                }
            }
        }

        Ok(RecoveryReport {
            recovered,
            failed,
            spawn_tree,
            dirty_sessions,
        })
    }

    /// Recover a single session
    async fn recover_session(&self, session_id: &str) -> Result<(), PersistenceError> {
        let checkpoint = self
            .storage
            .load_checkpoint(session_id)
            .await?
            .ok_or_else(|| PersistenceError::NotFound(session_id.to_string()))?;

        // Use the pre-stored recovery_notification from the checkpoint
        // (set by inject_recovery_notifications) when available;
        // otherwise fall back to building it fresh.
        let notification = if let Some(ref stored) = checkpoint.recovery_notification {
            Some(stored.clone())
        } else if !checkpoint.pending_operations.is_empty() {
            Some(self.build_notification_text(&checkpoint))
        } else {
            None
        };

        // Use pre-stored pending_tool_failures when available;
        // otherwise build them fresh.
        let tool_failures = if !checkpoint.pending_tool_failures.is_empty() {
            checkpoint.pending_tool_failures.clone()
        } else {
            self.build_tool_failure_results(&checkpoint)
        };

        let restore_fn = self.restore_fn.read().await;
        if let Some(callback) = restore_fn.as_ref() {
            callback(
                session_id,
                &checkpoint,
                notification.as_deref(),
                &tool_failures,
            )?;
        }

        Ok(())
    }

    /// Build the notification text for a dirty session.
    ///
    /// Stores the recovery notification and tool failure results in the
    /// checkpoint so they can be read back when sessions are restored.
    fn inject_recovery_notifications(&self, session_id: &str, checkpoint: &mut SessionCheckpoint) {
        if checkpoint.pending_operations.is_empty() {
            return;
        }

        let notification = self.build_notification_text(checkpoint);
        checkpoint.recovery_notification = Some(notification);
        checkpoint.pending_tool_failures = self.build_tool_failure_results(checkpoint);

        tracing::info!(
            session_id = %session_id,
            pending_count = checkpoint.pending_operations.len(),
            "storing recovery notification in checkpoint"
        );
    }

    /// Build notification text listing pending operations.
    fn build_notification_text(&self, checkpoint: &SessionCheckpoint) -> String {
        use crate::persistence::PendingOperationType;

        let restart_time = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");

        // Build summary by op_type
        let mut tool_calls = Vec::new();
        let mut sub_spawns = Vec::new();
        let mut outbound_msgs = Vec::new();

        for op in &checkpoint.pending_operations {
            match op.op_type {
                PendingOperationType::ToolCall => {
                    tool_calls.push(format!(
                        "  • 工具调用: {}({}) — 发起于 {}",
                        op.name,
                        if op.args.is_empty() {
                            "无参数".to_string()
                        } else {
                            op.args.clone()
                        },
                        op.created_at.format("%Y-%m-%dT%H:%M:%SZ")
                    ));
                }
                PendingOperationType::SubSessionSpawn => {
                    sub_spawns.push(format!(
                        "  • 子 Session: {} — 发起于 {}",
                        op.name,
                        op.created_at.format("%Y-%m-%dT%H:%M:%SZ")
                    ));
                }
                PendingOperationType::OutboundMessage => {
                    outbound_msgs.push(format!(
                        "  • 出站消息: {} — 创建于 {}",
                        op.name,
                        op.created_at.format("%Y-%m-%dT%H:%M:%SZ")
                    ));
                }
            }
        }

        let mut sections = Vec::new();
        if !tool_calls.is_empty() {
            sections.push(tool_calls.join("\n"));
        }
        if !sub_spawns.is_empty() {
            sections.push(sub_spawns.join("\n"));
        }
        if !outbound_msgs.is_empty() {
            sections.push(outbound_msgs.join("\n"));
        }

        format!(
            "[系统] 网关已重启（重启时间: {restart_time}）\n\n\
以下操作在重启前未完成：\n{ops}\n\n\
你可以使用 sessions_list、sessions_history、process 等工具\n\
了解当前状态，自行判断这些操作的结果，并决定后续处理。",
            restart_time = restart_time,
            ops = sections.join("\n\n"),
        )
    }

    /// Build tool failure result strings for pending tool call operations.
    fn build_tool_failure_results(&self, checkpoint: &SessionCheckpoint) -> Vec<String> {
        use crate::persistence::PendingOperationType;

        checkpoint
            .pending_operations
            .iter()
            .filter(|op| op.op_type == PendingOperationType::ToolCall)
            .map(|op| {
                serde_json::json!({
                    "error": "进程中断：网关重启",
                    "tool": op.name,
                    "op_id": op.op_id,
                })
                .to_string()
            })
            .collect()
    }

    /// 根据已恢复 session 的 checkpoint 构建 spawn_tree。
    ///
    /// - 有 `parent_session_id` 且父 session 已恢复 → 注册为父节点的子节点
    /// - 有 `parent_session_id` 但父 session 未恢复 → 降级为根节点，depth 重置为 0
    /// - 无 `parent_session_id` → 根节点
    fn build_spawn_tree(
        checkpoints: &mut HashMap<String, SessionCheckpoint>,
        recovered: &[String],
    ) -> (SpawnTree, Vec<String>) {
        let mut tree = SpawnTree::default();
        let mut demoted = Vec::new();
        let recovered_set: HashSet<&String> = recovered.iter().collect();

        for session_id in recovered {
            if let Some(cp) = checkpoints.get_mut(session_id) {
                match &cp.parent_session_id {
                    Some(parent_id) if recovered_set.contains(parent_id) => {
                        // 父 session 已恢复 — 注册为子节点
                        tree.children
                            .entry(parent_id.clone())
                            .or_default()
                            .push(session_id.clone());
                    }
                    Some(parent_id) => {
                        // 父 session 未恢复 — 降级为根节点，depth 重置为 0
                        tracing::info!(
                            session_id = %session_id,
                            parent_id = %parent_id,
                            "Session demoted to root: parent not recovered"
                        );
                        cp.depth = 0;
                        demoted.push(session_id.clone());
                        tree.roots.push(session_id.clone());
                    }
                    None => {
                        // 无父节点 — 确认为根节点
                        tree.roots.push(session_id.clone());
                    }
                }
            }
        }

        (tree, demoted)
    }

    /// Get the storage reference
    pub fn storage(&self) -> &S {
        &self.storage
    }
}

/// Spawn tree — tracks parent-child relationships between sessions.
///
/// Built during recovery from checkpoint data. Used by the Session module
/// to reconstruct the runtime spawn tree after gateway restart.
#[derive(Debug, Clone, Default)]
pub struct SpawnTree {
    /// parent_session_id → list of child session_ids
    pub children: HashMap<String, Vec<String>>,
    /// All root sessions (no parent or parent not recovered)
    pub roots: Vec<String>,
}

impl SpawnTree {
    /// Check if a session is a root node (no parent or parent not recovered).
    pub fn is_root(&self, session_id: &str) -> bool {
        self.roots.iter().any(|id| id == session_id)
    }

    /// Get children of a session.
    pub fn get_children(&self, session_id: &str) -> Option<&Vec<String>> {
        self.children.get(session_id)
    }

    /// Get all root session IDs.
    pub fn root_ids(&self) -> &[String] {
        &self.roots
    }

    /// Get the parent session ID of a given session.
    ///
    /// Returns `None` for root nodes or unknown sessions.
    pub fn get_parent(&self, session_id: &str) -> Option<&String> {
        if self.is_root(session_id) {
            return None;
        }
        self.children
            .iter()
            .find(|(_, children)| children.iter().any(|id| id == session_id))
            .map(|(parent, _)| parent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::{
        DreamingStatus, PendingOperation, PendingOperationType, ReasoningLevel, ReasoningMode,
        ReasoningModeState, SessionStatus,
    };
    use crate::storage::memory::MemoryStorage;
    use chrono::Utc;

    fn create_test_checkpoint(session_id: &str) -> SessionCheckpoint {
        SessionCheckpoint {
            session_id: session_id.to_string(),
            last_message_id: Some("msg123".to_string()),
            mode_state: ReasoningModeState {
                current_step: 1,
                total_steps: 3,
                step_messages: vec!["Step 1".to_string()],
                is_complete: false,
            },
            pending_messages: Vec::new(),
            mode: ReasoningMode::Plan,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            ttl_seconds: 604800,
            status: SessionStatus::Active,
            last_message_at: None,
            message_count: 0,
            platform: None,
            peer_id: None,
            account_id: None,
            agent_id: None,
            role: None,
            reasoning_level: ReasoningLevel::default(),
            system_appends: Vec::new(),
            thread_id: None,
            sender_id: None,
            parent_session_id: None,
            depth: 0,
            effective_max_spawn_depth: None,
            mined: false,
            dreaming_status: DreamingStatus::default(),
            pending_operations: Vec::new(),
            recovery_notification: None,
            pending_tool_failures: Vec::new(),
            verbosity_level: closeclaw_common::VerbosityLevel::default(),
        }
    }

    #[tokio::test]
    async fn test_recovery_report_is_full_success() {
        let report = RecoveryReport {
            recovered: vec!["s1".to_string(), "s2".to_string()],
            failed: Vec::new(),
            spawn_tree: SpawnTree::default(),
            dirty_sessions: Vec::new(),
        };
        assert!(report.is_full_success());
        assert_eq!(report.total(), 2);
    }

    #[tokio::test]
    async fn test_recovery_report_has_failures() {
        let report = RecoveryReport {
            recovered: vec!["s1".to_string()],
            failed: vec!["s2".to_string()],
            spawn_tree: SpawnTree::default(),
            dirty_sessions: Vec::new(),
        };
        assert!(!report.is_full_success());
        assert_eq!(report.total(), 2);
    }

    #[tokio::test]
    async fn test_recovery_service_recover_empty() {
        let storage = Arc::new(MemoryStorage::new());
        let service = SessionRecoveryService::new(storage);

        let report = service.recover().await.unwrap();
        assert!(report.recovered.is_empty());
        assert!(report.failed.is_empty());
    }

    #[tokio::test]
    async fn test_recovery_service_recover_with_callback() {
        let storage = Arc::new(MemoryStorage::new());

        // Pre-populate storage with checkpoints
        storage
            .save_checkpoint(&create_test_checkpoint("session1"))
            .await
            .unwrap();
        storage
            .save_checkpoint(&create_test_checkpoint("session2"))
            .await
            .unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));

        // Track which sessions were restored
        let restored = Arc::new(std::sync::Mutex::new(Vec::new()));
        let restored_clone = Arc::clone(&restored);

        service
            .set_restore_callback(
                move |session_id, _checkpoint, _notification, _tool_failures| {
                    restored_clone.lock().unwrap().push(session_id.to_string());
                    Ok(())
                },
            )
            .await;

        let report = service.recover().await.unwrap();

        assert_eq!(report.recovered.len(), 2);
        assert!(report.failed.is_empty());

        let mut restored_sessions = restored.lock().unwrap();
        restored_sessions.sort();
        assert_eq!(restored_sessions[0], "session1");
        assert_eq!(restored_sessions[1], "session2");
    }

    #[tokio::test]
    async fn test_recovery_service_recover_not_found() {
        let storage = Arc::new(MemoryStorage::new());
        let service = SessionRecoveryService::new(Arc::clone(&storage));

        // Don't set any restore callback, but storage has a checkpoint
        storage
            .save_checkpoint(&create_test_checkpoint("orphan"))
            .await
            .unwrap();

        // Recover should still succeed even without callback
        let report = service.recover().await.unwrap();
        assert_eq!(report.recovered.len(), 1);
    }

    // -----------------------------------------------------------------
    // Spawn tree tests
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_recovery_spawn_tree_root_sessions() {
        let storage = Arc::new(MemoryStorage::new());
        storage
            .save_checkpoint(&create_test_checkpoint("root1"))
            .await
            .unwrap();
        storage
            .save_checkpoint(&create_test_checkpoint("root2"))
            .await
            .unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();

        assert_eq!(report.recovered.len(), 2);
        let tree = &report.spawn_tree;
        assert_eq!(tree.roots.len(), 2);
        assert!(tree.roots.contains(&"root1".to_string()));
        assert!(tree.roots.contains(&"root2".to_string()));
        assert!(tree.children.is_empty());
    }

    #[tokio::test]
    async fn test_recovery_spawn_tree_parent_child() {
        let storage = Arc::new(MemoryStorage::new());

        // Parent session
        let mut parent_cp = create_test_checkpoint("parent");
        parent_cp.parent_session_id = None;
        parent_cp.depth = 0;
        storage.save_checkpoint(&parent_cp).await.unwrap();

        // Child session
        let mut child_cp = create_test_checkpoint("child");
        child_cp.parent_session_id = Some("parent".to_string());
        child_cp.depth = 1;
        storage.save_checkpoint(&child_cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();

        assert_eq!(report.recovered.len(), 2);
        let tree = &report.spawn_tree;

        // Parent is root, child is registered under parent
        assert!(tree.is_root("parent"));
        assert!(!tree.is_root("child"));
        let children = tree.get_children("parent").unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0], "child");
    }

    #[tokio::test]
    async fn test_recovery_spawn_tree_orphan_demoted_to_root() {
        let storage = Arc::new(MemoryStorage::new());

        // Child session whose parent is NOT in storage (swept)
        let mut child_cp = create_test_checkpoint("orphan_child");
        child_cp.parent_session_id = Some("missing_parent".to_string());
        child_cp.depth = 2;
        storage.save_checkpoint(&child_cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();

        assert_eq!(report.recovered.len(), 1);
        let tree = &report.spawn_tree;

        // Orphan child is demoted to root
        assert!(tree.is_root("orphan_child"));
        assert!(tree.children.is_empty());
    }

    #[tokio::test]
    async fn test_recovery_spawn_tree_multi_level() {
        let storage = Arc::new(MemoryStorage::new());

        // root -> child1 -> grandchild
        let mut root_cp = create_test_checkpoint("root");
        root_cp.parent_session_id = None;
        root_cp.depth = 0;
        storage.save_checkpoint(&root_cp).await.unwrap();

        let mut child_cp = create_test_checkpoint("child1");
        child_cp.parent_session_id = Some("root".to_string());
        child_cp.depth = 1;
        storage.save_checkpoint(&child_cp).await.unwrap();

        let mut grandchild_cp = create_test_checkpoint("grandchild");
        grandchild_cp.parent_session_id = Some("child1".to_string());
        grandchild_cp.depth = 2;
        storage.save_checkpoint(&grandchild_cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();

        assert_eq!(report.recovered.len(), 3);
        let tree = &report.spawn_tree;

        assert!(tree.is_root("root"));
        assert!(!tree.is_root("child1"));
        assert!(!tree.is_root("grandchild"));

        let root_children = tree.get_children("root").unwrap();
        assert_eq!(root_children, &vec!["child1".to_string()]);

        let child1_children = tree.get_children("child1").unwrap();
        assert_eq!(child1_children, &vec!["grandchild".to_string()]);
    }

    #[test]
    fn test_spawn_tree_is_root() {
        let tree = SpawnTree {
            roots: vec!["r1".to_string(), "r2".to_string()],
            children: HashMap::new(),
        };
        assert!(tree.is_root("r1"));
        assert!(tree.is_root("r2"));
        assert!(!tree.is_root("r3"));
    }

    #[test]
    fn test_spawn_tree_get_children() {
        let mut children = HashMap::new();
        children.insert("p1".to_string(), vec!["c1".to_string(), "c2".to_string()]);
        let tree = SpawnTree {
            roots: vec![],
            children,
        };
        assert_eq!(tree.get_children("p1").unwrap().len(), 2);
        assert!(tree.get_children("p2").is_none());
    }

    #[test]
    fn test_spawn_tree_root_ids() {
        let tree = SpawnTree {
            roots: vec!["a".to_string(), "b".to_string()],
            children: HashMap::new(),
        };
        assert_eq!(tree.root_ids(), &["a", "b"]);
    }

    #[test]
    fn test_build_spawn_tree_demoted_depth_reset() {
        // orphan child with depth=2 should be demoted to root with depth=0
        let mut checkpoints = HashMap::new();
        let mut orphan_cp = create_test_checkpoint("orphan");
        orphan_cp.parent_session_id = Some("missing_parent".to_string());
        orphan_cp.depth = 2;
        checkpoints.insert("orphan".to_string(), orphan_cp);

        let recovered = vec!["orphan".to_string()];
        let (tree, demoted) =
            SessionRecoveryService::<MemoryStorage>::build_spawn_tree(&mut checkpoints, &recovered);

        assert!(tree.is_root("orphan"));
        assert_eq!(checkpoints["orphan"].depth, 0);
        assert!(demoted.contains(&"orphan".to_string()));
    }

    #[test]
    fn test_build_spawn_tree_empty() {
        let mut checkpoints = HashMap::new();
        let recovered: Vec<String> = vec![];
        let (tree, demoted) =
            SessionRecoveryService::<MemoryStorage>::build_spawn_tree(&mut checkpoints, &recovered);
        assert!(tree.roots.is_empty());
        assert!(tree.children.is_empty());
        assert!(demoted.is_empty());
    }

    #[test]
    fn test_build_spawn_tree_partial_recovery() {
        // parent recovered, child NOT recovered → child not in tree at all
        let mut checkpoints = HashMap::new();
        let mut parent_cp = create_test_checkpoint("parent");
        parent_cp.parent_session_id = None;
        checkpoints.insert("parent".to_string(), parent_cp);

        let recovered = vec!["parent".to_string()];
        let (tree, demoted) =
            SessionRecoveryService::<MemoryStorage>::build_spawn_tree(&mut checkpoints, &recovered);
        assert_eq!(tree.roots, vec!["parent".to_string()]);
        assert!(tree.children.is_empty());
        assert!(demoted.is_empty());
    }

    #[tokio::test]
    async fn test_recovery_notification_stored_in_checkpoint() {
        let storage = Arc::new(MemoryStorage::new());
        let now = Utc::now();
        let cp = SessionCheckpoint::new("dirty_notif".into()).with_pending_operations(vec![
            PendingOperation {
                op_id: "op_1".into(),
                op_type: PendingOperationType::ToolCall,
                name: "bash".into(),
                args: r#"{"cmd":"ls"}"#.into(),
                created_at: now,
            },
        ]);
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();

        assert_eq!(report.recovered.len(), 1);
        assert!(report.dirty_sessions.contains(&"dirty_notif".to_string()));

        // Verify notification was stored in checkpoint
        let loaded = storage
            .load_checkpoint("dirty_notif")
            .await
            .unwrap()
            .unwrap();
        assert!(
            loaded.recovery_notification.is_some(),
            "recovery_notification should be stored in dirty checkpoint"
        );
        let notif = loaded.recovery_notification.unwrap();
        assert!(notif.contains("网关已重启"));
        assert!(notif.contains("工具调用: bash"));
    }

    #[tokio::test]
    async fn test_recovery_notification_not_stored_for_clean_session() {
        let storage = Arc::new(MemoryStorage::new());
        let cp = SessionCheckpoint::new("clean_notif".into());
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();

        assert_eq!(report.recovered.len(), 1);
        assert!(report.dirty_sessions.is_empty());

        let loaded = storage
            .load_checkpoint("clean_notif")
            .await
            .unwrap()
            .unwrap();
        assert!(loaded.recovery_notification.is_none());
    }

    #[tokio::test]
    async fn test_recovery_tool_failures_built() {
        let storage = Arc::new(MemoryStorage::new());
        let now = Utc::now();
        let cp = SessionCheckpoint::new("dirty_tools".into()).with_pending_operations(vec![
            PendingOperation {
                op_id: "call_1".into(),
                op_type: PendingOperationType::ToolCall,
                name: "exec".into(),
                args: r#"{"command":"kubectl get pods"}"#.into(),
                created_at: now,
            },
            PendingOperation {
                op_id: "child_1".into(),
                op_type: PendingOperationType::SubSessionSpawn,
                name: "sub-agent".into(),
                args: String::new(),
                created_at: now,
            },
        ]);
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let _report = service.recover().await.unwrap();

        let loaded = storage
            .load_checkpoint("dirty_tools")
            .await
            .unwrap()
            .unwrap();
        // Only ToolCall ops produce failure results
        assert_eq!(loaded.pending_tool_failures.len(), 1);
        let failure = &loaded.pending_tool_failures[0];
        assert!(failure.contains("进程中断：网关重启"));
        assert!(failure.contains("exec"));
        assert!(failure.contains("call_1"));
    }

    #[tokio::test]
    async fn test_recovery_callback_receives_notification() {
        use crate::persistence::{
            PendingOperation, PendingOperationType, PersistenceService, SessionCheckpoint,
        };
        use crate::storage::memory::MemoryStorage;
        use std::sync::Arc;

        let storage = Arc::new(MemoryStorage::new());
        let now = Utc::now();
        let cp = SessionCheckpoint::new("cb_notif".into()).with_pending_operations(vec![
            PendingOperation {
                op_id: "op_1".into(),
                op_type: PendingOperationType::ToolCall,
                name: "bash".into(),
                args: String::new(),
                created_at: now,
            },
        ]);
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));

        let captured_notification = Arc::new(std::sync::Mutex::new(None::<String>));
        let captured_failures = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let notif_clone = Arc::clone(&captured_notification);
        let fail_clone = Arc::clone(&captured_failures);

        service
            .set_restore_callback(
                move |_session_id, _checkpoint, notification, tool_failures| {
                    *notif_clone.lock().unwrap() = notification.map(String::from);
                    *fail_clone.lock().unwrap() = tool_failures.to_vec();
                    Ok(())
                },
            )
            .await;

        let report = service.recover().await.unwrap();
        assert_eq!(report.recovered.len(), 1);

        let notif = captured_notification.lock().unwrap();
        assert!(notif.is_some(), "callback should receive notification text");
        let notif_text = notif.as_ref().unwrap();
        assert!(notif_text.contains("网关已重启"));
        assert!(notif_text.contains("工具调用: bash"));

        let failures = captured_failures.lock().unwrap();
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("进程中断：网关重启"));
    }
}
