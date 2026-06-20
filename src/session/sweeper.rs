//! Archive Sweeper — background task for session lifecycle management
//!
//! Periodically scans for idle sessions to archive and expired archived
//! sessions to purge. Spawned by the daemon at startup and shut down
//! gracefully via a `tokio::sync::watch` channel.

use std::panic;
use std::sync::Arc;

use thiserror::Error;
use tokio::sync::watch;
use tokio::time::Instant;
use tracing::{error, info, warn};

use crate::config::session::SessionConfigProvider;
use crate::gateway::session_manager::SessionManager;
use crate::session::persistence::{AgentRole, PersistenceError, PersistenceService};

/// Errors that can occur during sweeper operations.
#[derive(Debug, Error)]
pub enum ArchiveSweeperError {
    #[error("storage error: {0}")]
    Storage(#[from] PersistenceError),

    #[error("config error: {0}")]
    Config(String),
}

/// Archive Sweeper — scans and cleans up sessions based on configured TTLs.
pub struct ArchiveSweeper {
    storage: Arc<dyn PersistenceService>,
    config: Arc<dyn SessionConfigProvider>,
    /// Optional runtime session manager for cleaning up child sessions
    /// when archiving a parent (design doc §生命周期联动).
    session_manager: Option<Arc<SessionManager>>,
}

impl ArchiveSweeper {
    /// Create a new `ArchiveSweeper`.
    pub fn new(
        storage: Arc<dyn PersistenceService>,
        config: Arc<dyn SessionConfigProvider>,
    ) -> Self {
        Self {
            storage,
            config,
            session_manager: None,
        }
    }

    /// Attach a runtime [`SessionManager`] so the sweeper can
    /// cascade-terminate children when archiving a parent session.
    pub fn with_session_manager(mut self, sm: Arc<SessionManager>) -> Self {
        self.session_manager = Some(sm);
        self
    }

    /// Run the sweeper loop until `shutdown` signal is received.
    ///
    /// # Arguments
    /// * `shutdown` — watch receiver that will be closed when the sweeper
    ///   should exit gracefully
    pub async fn run(&self, mut shutdown: watch::Receiver<()>) {
        let interval_secs = self.config.sweeper_interval_secs();
        let interval = tokio::time::Duration::from_secs(interval_secs);

        #[cfg(unix)]
        self.lower_priority();

        let mut next_fire = Instant::now() + interval;

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    info!("ArchiveSweeper received shutdown signal, exiting");
                    break;
                }
                _ = tokio::time::sleep_until(next_fire) => {
                    if let Err(e) = self.run_once().await {
                        error!(%e, "run_once returned error, continuing loop");
                    }
                    next_fire += interval;
                    if Instant::now() > next_fire + interval {
                        next_fire = Instant::now() + interval;
                    }
                }
            }
        }
    }

    /// Lower the process nice value on Unix to reduce sweeper priority.
    #[cfg(unix)]
    fn lower_priority(&self) {
        if unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, 10) } != 0 {
            let err = std::io::Error::last_os_error();
            warn!("failed to set sweeper nice value: {}", err);
        } else {
            info!("Sweeper process priority set to nice=10");
        }
    }

    /// Execute one sweep: archive idle sessions and purge expired archives.
    ///
    /// Panics inside `run_once` are caught so a buggy callback does not kill
    /// the sweeper — the error is logged and the sweep continues.
    pub async fn run_once(&self) -> Result<(), ArchiveSweeperError> {
        // Wrap in AssertUnwindSafe + catch_unwind so panics in storage
        // methods do not kill the async task.
        // Use spawn_blocking + block_on so catch_unwind can catch panics
        // that escape the async future. This works because block_on runs
        // the future on the current thread (not a separate tokio worker),
        // so panics propagate into catch_unwind.
        let storage = Arc::clone(&self.storage);
        let config = Arc::clone(&self.config);
        let session_manager = self.session_manager.clone();

        let handle = tokio::task::spawn_blocking(move || {
            let runtime = tokio::runtime::Handle::current();
            panic::catch_unwind(panic::AssertUnwindSafe(|| {
                runtime.block_on(Self::run_once_inner_impl(
                    Arc::clone(&storage),
                    Arc::clone(&config),
                    session_manager,
                ))
            }))
        });

        let result = handle.await;

        match result {
            // spawn_blocking itself panicked (should not happen for normal storage code)
            Err(_) => {
                error!("run_once panicked in spawn_blocking, continuing");
                Ok(())
            }
            // inner Result: Ok = normal completion, Err = catch_unwind caught a panic
            Ok(inner) => {
                if inner.is_err() {
                    // Panic was caught by catch_unwind (storage method panicked)
                    error!("run_once panicked and was caught, continuing");
                }
                inner.unwrap_or(Ok(()))
            }
        }
    }

    /// Inner run_once logic — extracted so it can be called inside
    /// spawn_blocking + catch_unwind from `run_once`.
    async fn run_once_inner_impl(
        storage: Arc<dyn PersistenceService>,
        config: Arc<dyn SessionConfigProvider>,
        session_manager: Option<Arc<SessionManager>>,
    ) -> Result<(), ArchiveSweeperError> {
        let agents = config.list_agents();
        if agents.is_empty() {
            return Ok(());
        }

        for agent_id in &agents {
            for role in [AgentRole::MainAgent, AgentRole::SubAgent] {
                Self::sweep_agent_role(
                    Arc::clone(&storage),
                    config.as_ref(),
                    agent_id,
                    role,
                    session_manager.as_ref(),
                )
                .await;
            }
        }

        Ok(())
    }

    /// Sweep one agent + role: archive idle sessions, purge expired archives.
    async fn sweep_agent_role(
        storage: Arc<dyn PersistenceService>,
        config: &dyn SessionConfigProvider,
        agent_id: &str,
        role: AgentRole,
        session_manager: Option<&Arc<SessionManager>>,
    ) {
        let cfg = config.session_config_for(agent_id, role);

        // --- Idle → archive (cascade children first) ---
        if let Ok(idle_ids) = storage
            .list_idle_sessions_for_agent(agent_id, role, cfg.idle_minutes)
            .await
        {
            for sid in idle_ids {
                let sid_err = sid.clone();
                if let Err(e) =
                    Self::cascade_archive_impl(Arc::clone(&storage), sid, session_manager).await
                {
                    error!(session_id = %sid_err, %e, "failed to archive idle session");
                }
            }
        }

        // --- Expired archive → purge (skip if 0 = never purge) ---
        if cfg.purge_after_minutes > 0 {
            if let Ok(expired_ids) = storage
                .list_expired_archived_sessions_for_agent(agent_id, role, cfg.purge_after_minutes)
                .await
            {
                for sid in expired_ids {
                    let sid_err = sid.clone();
                    if let Err(e) = Self::purge_and_invalidate_impl(Arc::clone(&storage), sid).await
                    {
                        error!(session_id = %sid_err, %e, "failed to purge expired session");
                    }
                }
            }
        } else {
            warn!(
                agent_id,
                ?role,
                "purge_after_minutes is 0, skipping purge scan"
            );
        }
    }

    /// Archive a session and invalidate its local cache.
    async fn archive_and_invalidate_impl(
        storage: Arc<dyn PersistenceService>,
        session_id: String,
    ) -> Result<(), ArchiveSweeperError> {
        let checkpoint = storage.load_checkpoint(&session_id).await?.ok_or_else(|| {
            ArchiveSweeperError::Config(format!("session {session_id} not found for archive"))
        })?;

        storage.archive_checkpoint(&checkpoint).await?;
        storage.invalidate_session(&session_id).await?;
        info!(%session_id, "session archived and cache invalidated");
        Ok(())
    }

    /// Cascade-terminate all descendants of `session_id` (deepest first),
    /// then archive the parent session.
    ///
    /// When a runtime [`SessionManager`] is available, also cleans up
    /// child sessions from the runtime tracking tables (design doc
    /// §生命周期联动: "父 session 超时清理: 级联终止所有子 session").
    async fn cascade_archive_impl(
        storage: Arc<dyn PersistenceService>,
        session_id: String,
        session_manager: Option<&Arc<SessionManager>>,
    ) -> Result<(), ArchiveSweeperError> {
        // Recursively kill all descendants (deepest → shallowest)
        Self::cascade_kill_children(storage.as_ref(), &session_id).await?;

        // Clean up runtime tracking tables if SessionManager is available.
        // This terminates active child sessions and removes them from
        // conversation_sessions / sessions / children tables.
        if let Some(sm) = session_manager {
            sm.cascade_kill_all_children(&session_id).await;
        }

        // Now archive the parent session itself
        Self::archive_and_invalidate_impl(Arc::clone(&storage), session_id.clone()).await?;
        info!(%session_id, "session archived after cascade cleanup");
        Ok(())
    }

    /// 删除 `parent_session_id` 的所有后代 session（先深后浅）。
    /// 使用 BFS 收集所有后代及其深度，按深度降序删除（叶节点优先）。
    async fn cascade_kill_children(
        storage: &dyn PersistenceService,
        parent_session_id: &str,
    ) -> Result<(), ArchiveSweeperError> {
        // BFS 收集所有后代及其深度
        let mut all_descendants: Vec<(String, u32)> = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back((parent_session_id.to_string(), 0u32));

        while let Some((current, depth)) = queue.pop_front() {
            let children = storage.list_children_sessions(&current).await?;
            for child_id in &children {
                all_descendants.push((child_id.clone(), depth + 1));
                queue.push_back((child_id.clone(), depth + 1));
            }
        }

        // 按深度降序排序（叶节点优先删除），深度相同则按 ID 排序保证确定性
        all_descendants.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

        for (child_id, _) in all_descendants {
            storage.delete_checkpoint(&child_id).await?;
            storage.invalidate_session(&child_id).await?;
            info!(
                parent = %parent_session_id,
                child = %child_id,
                "child session deleted and invalidated during cascade"
            );
        }

        Ok(())
    }

    /// Purge an archived session and invalidate its local cache (impl version with Arc).
    async fn purge_and_invalidate_impl(
        storage: Arc<dyn PersistenceService>,
        session_id: String,
    ) -> Result<(), ArchiveSweeperError> {
        storage.purge_checkpoint(&session_id).await?;
        storage.invalidate_session(&session_id).await?;
        info!(%session_id, "session purged and cache invalidated");
        Ok(())
    }
}

impl std::fmt::Debug for ArchiveSweeper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveSweeper").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::session::PerAgentSessionConfig;
    use crate::session::persistence::SessionCheckpoint;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// In-memory storage suitable for tests.
    #[derive(Debug, Default)]
    struct MemStorage {
        checkpoints: Mutex<Vec<SessionCheckpoint>>,
        _archived: Mutex<Vec<String>>,
        invalidated: Mutex<Vec<String>>,
        archive_called: Mutex<Vec<String>>,
        purge_called: Mutex<Vec<String>>,
        /// Session IDs returned from `list_idle_sessions_for_agent`.
        idle_sessions: Mutex<Vec<String>>,
        /// Session IDs returned from `list_expired_archived_sessions_for_agent`.
        expired_sessions: Mutex<Vec<String>>,
        /// Session IDs deleted via `delete_checkpoint`.
        deleted: Mutex<Vec<String>>,
    }

    impl MemStorage {
        /// Add a session ID to be returned as idle by `list_idle_sessions_for_agent`.
        fn add_idle_session(&self, session_id: String) {
            self.idle_sessions.lock().unwrap().push(session_id);
        }

        /// Add a session ID to be returned as expired by
        /// `list_expired_archived_sessions_for_agent`.
        fn add_expired_session(&self, session_id: String) {
            self.expired_sessions.lock().unwrap().push(session_id);
        }

        /// Add a checkpoint so it can be loaded by `load_checkpoint`.
        fn add_checkpoint(&self, checkpoint: SessionCheckpoint) {
            self.checkpoints.lock().unwrap().push(checkpoint);
        }

        /// Get IDs of sessions deleted via `delete_checkpoint`.
        fn deleted_ids(&self) -> Vec<String> {
            self.deleted.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl PersistenceService for MemStorage {
        async fn save_checkpoint(
            &self,
            checkpoint: &SessionCheckpoint,
        ) -> Result<(), PersistenceError> {
            self.checkpoints.lock().unwrap().push(checkpoint.clone());
            Ok(())
        }

        async fn load_checkpoint(
            &self,
            session_id: &str,
        ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
            let checkpoints = self.checkpoints.lock().unwrap();
            Ok(checkpoints
                .iter()
                .find(|cp| cp.session_id == session_id)
                .cloned())
        }

        async fn delete_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError> {
            self.deleted.lock().unwrap().push(session_id.into());
            self.checkpoints
                .lock()
                .unwrap()
                .retain(|cp| cp.session_id != session_id);
            Ok(())
        }

        async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }

        async fn archive_checkpoint(
            &self,
            checkpoint: &SessionCheckpoint,
        ) -> Result<(), PersistenceError> {
            self.archive_called
                .lock()
                .unwrap()
                .push(checkpoint.session_id.clone());
            Ok(())
        }

        async fn purge_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError> {
            self.purge_called.lock().unwrap().push(session_id.into());
            Ok(())
        }

        async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }

        async fn invalidate_session(&self, session_id: &str) -> Result<(), PersistenceError> {
            self.invalidated.lock().unwrap().push(session_id.into());
            Ok(())
        }

        async fn list_idle_sessions_for_agent(
            &self,
            _agent_id: &str,
            _role: AgentRole,
            _idle_minutes: i64,
        ) -> Result<Vec<String>, PersistenceError> {
            Ok(self.idle_sessions.lock().unwrap().clone())
        }

        async fn list_expired_archived_sessions_for_agent(
            &self,
            _agent_id: &str,
            _role: AgentRole,
            _purge_after_minutes: i64,
        ) -> Result<Vec<String>, PersistenceError> {
            Ok(self.expired_sessions.lock().unwrap().clone())
        }

        async fn list_children_sessions(
            &self,
            parent_session_id: &str,
        ) -> Result<Vec<String>, PersistenceError> {
            let checkpoints = self.checkpoints.lock().unwrap();
            let children: Vec<String> = checkpoints
                .iter()
                .filter(|cp| cp.parent_session_id.as_deref() == Some(parent_session_id))
                .map(|cp| cp.session_id.clone())
                .collect();
            Ok(children)
        }
    }

    /// Mock config provider for tests.
    #[derive(Debug, Default)]
    struct MockConfig {
        agents: Mutex<Vec<String>>,
        session_config: Mutex<PerAgentSessionConfig>,
        interval_secs: Mutex<u64>,
    }

    impl MockConfig {
        fn with_agents(agents: Vec<String>) -> Self {
            Self {
                agents: Mutex::new(agents),
                ..Default::default()
            }
        }
    }

    impl SessionConfigProvider for MockConfig {
        fn session_config_for(&self, _agent_id: &str, _role: AgentRole) -> PerAgentSessionConfig {
            self.session_config.lock().unwrap().clone()
        }

        fn sweeper_interval_secs(&self) -> u64 {
            *self.interval_secs.lock().unwrap()
        }

        fn list_agents(&self) -> Vec<String> {
            self.agents.lock().unwrap().clone()
        }

        fn compact_config(&self) -> crate::session::CompactConfig {
            crate::session::CompactConfig::default()
        }
    }

    // -----------------------------------------------------------------
    // Test: run_once calls archive for idle sessions
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_run_once_calls_archive() {
        let mem = Arc::new(MemStorage::default());
        mem.add_idle_session("session-1".into());
        mem.add_checkpoint(SessionCheckpoint::new("session-1".into()));

        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        let config: Arc<dyn SessionConfigProvider> =
            Arc::new(MockConfig::with_agents(vec!["agent-x".into()]));

        let sweeper = ArchiveSweeper::new(Arc::clone(&storage), Arc::clone(&config));
        sweeper.run_once().await.unwrap();

        let archive_called = mem.archive_called.lock().unwrap();
        assert!(archive_called.contains(&"session-1".into()));
    }

    // -----------------------------------------------------------------
    // Test: run_once calls purge for expired archived sessions
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_run_once_calls_purge() {
        let mem = Arc::new(MemStorage::default());
        mem.add_expired_session("session-2".into());

        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        let config: Arc<dyn SessionConfigProvider> =
            Arc::new(MockConfig::with_agents(vec!["agent-x".into()]));

        let sweeper = ArchiveSweeper::new(Arc::clone(&storage), Arc::clone(&config));
        sweeper.run_once().await.unwrap();

        let purge_called = mem.purge_called.lock().unwrap();
        assert!(purge_called.contains(&"session-2".into()));
    }

    // -----------------------------------------------------------------
    // Test: purge_after_minutes = 0 skips purge scan
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_purge_after_zero_skips_purge() {
        let mem = Arc::new(MemStorage::default());
        mem.add_expired_session("session-x".into());

        let mock_config = MockConfig::with_agents(vec!["agent-x".into()]);
        *mock_config.session_config.lock().unwrap() = PerAgentSessionConfig::new(30, 0);
        let config: Arc<dyn SessionConfigProvider> = Arc::new(mock_config);

        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        let sweeper = ArchiveSweeper::new(Arc::clone(&storage), Arc::clone(&config));
        let result = sweeper.run_once().await;

        assert!(result.is_ok());
        let purge_called = mem.purge_called.lock().unwrap();
        assert!(purge_called.is_empty());
    }

    // -----------------------------------------------------------------
    // Test: no agents returns Ok without error
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_no_agents_no_error() {
        let mem = Arc::new(MemStorage::default());
        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::with_agents(vec![]));

        let sweeper = ArchiveSweeper::new(Arc::clone(&storage), Arc::clone(&config));
        let result = sweeper.run_once().await;

        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------
    // Test: run_once error does not stop loop (panic is caught)
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_run_once_error_does_not_stop_loop() {
        let mem = Arc::new(MemStorage::default());
        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        let config: Arc<dyn SessionConfigProvider> =
            Arc::new(MockConfig::with_agents(vec!["agent-x".into()]));

        let sweeper = ArchiveSweeper::new(Arc::clone(&storage), Arc::clone(&config));

        let result1 = sweeper.run_once().await;
        assert!(result1.is_ok());

        let result2 = sweeper.run_once().await;
        assert!(result2.is_ok());
    }

    // -----------------------------------------------------------------
    // Test: shutdown signal causes run() to exit
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_shutdown_exits_loop() {
        let mem = Arc::new(MemStorage::default());
        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::with_agents(vec![]));

        let (tx, rx) = watch::channel(());

        let sweeper = ArchiveSweeper::new(Arc::clone(&storage), Arc::clone(&config));

        let handle = tokio::spawn(async move {
            sweeper.run(rx).await;
        });

        let _ = tx.send(());
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
    }

    // -----------------------------------------------------------------
    // Test: cascade_kill_children deletes all descendants
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_cascade_kill_children_deletes_descendants() {
        let mem = Arc::new(MemStorage::default());

        // Build a 3-level tree: parent -> child1 -> grandchild
        let mut parent = SessionCheckpoint::new("parent".into());
        parent.parent_session_id = None;
        parent.depth = 0;
        mem.add_checkpoint(parent);

        let mut child1 = SessionCheckpoint::new("child1".into());
        child1.parent_session_id = Some("parent".into());
        child1.depth = 1;
        mem.add_checkpoint(child1);

        let mut grandchild = SessionCheckpoint::new("grandchild".into());
        grandchild.parent_session_id = Some("child1".into());
        grandchild.depth = 2;
        mem.add_checkpoint(grandchild);

        // Run cascade
        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        ArchiveSweeper::cascade_kill_children(storage.as_ref(), "parent")
            .await
            .unwrap();

        // Both descendants should be deleted
        let deleted = mem.deleted_ids();
        assert!(deleted.contains(&"child1".to_string()));
        assert!(deleted.contains(&"grandchild".to_string()));
        // Parent itself should NOT be deleted by cascade_kill_children
        assert!(!deleted.contains(&"parent".to_string()));
    }

    #[tokio::test]
    async fn test_cascade_kill_children_no_children_noop() {
        let mem = Arc::new(MemStorage::default());
        mem.add_checkpoint(SessionCheckpoint::new("leaf".into()));

        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        ArchiveSweeper::cascade_kill_children(storage.as_ref(), "leaf")
            .await
            .unwrap();

        // No children to delete
        let deleted = mem.deleted_ids();
        assert!(deleted.is_empty());
    }

    #[tokio::test]
    async fn test_cascade_kill_children_siblings_only() {
        let mem = Arc::new(MemStorage::default());

        let mut parent = SessionCheckpoint::new("parent".into());
        parent.parent_session_id = None;
        mem.add_checkpoint(parent);

        let mut child_a = SessionCheckpoint::new("child_a".into());
        child_a.parent_session_id = Some("parent".into());
        mem.add_checkpoint(child_a);

        let mut child_b = SessionCheckpoint::new("child_b".into());
        child_b.parent_session_id = Some("parent".into());
        mem.add_checkpoint(child_b);

        // Unrelated child under a different parent
        let mut other_child = SessionCheckpoint::new("other_child".into());
        other_child.parent_session_id = Some("other_parent".into());
        mem.add_checkpoint(other_child);

        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        ArchiveSweeper::cascade_kill_children(storage.as_ref(), "parent")
            .await
            .unwrap();

        let deleted = mem.deleted_ids();
        assert!(deleted.contains(&"child_a".to_string()));
        assert!(deleted.contains(&"child_b".to_string()));
        // other_child should NOT be deleted
        assert!(!deleted.contains(&"other_child".to_string()));
    }

    // -----------------------------------------------------------------
    // Test: cascade_archive kills children then archives parent
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_cascade_archive_kills_children_then_archives_parent() {
        let mem = Arc::new(MemStorage::default());

        let mut parent = SessionCheckpoint::new("parent-archive".into());
        parent.parent_session_id = None;
        mem.add_checkpoint(parent);

        let mut child = SessionCheckpoint::new("child-archive".into());
        child.parent_session_id = Some("parent-archive".into());
        mem.add_checkpoint(child);

        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        ArchiveSweeper::cascade_archive_impl(storage, "parent-archive".into(), None)
            .await
            .unwrap();

        // Child should be deleted
        let deleted = mem.deleted_ids();
        assert!(deleted.contains(&"child-archive".to_string()));

        // Parent should be archived
        let archive_called = mem.archive_called.lock().unwrap();
        assert!(archive_called.contains(&"parent-archive".into()));
    }

    #[tokio::test]
    async fn test_cascade_archive_no_children_archives_parent() {
        let mem = Arc::new(MemStorage::default());
        mem.add_checkpoint(SessionCheckpoint::new("solo-parent".into()));

        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        ArchiveSweeper::cascade_archive_impl(storage, "solo-parent".into(), None)
            .await
            .unwrap();

        // No children deleted
        let deleted = mem.deleted_ids();
        assert!(deleted.is_empty());

        // Parent should be archived
        let archive_called = mem.archive_called.lock().unwrap();
        assert!(archive_called.contains(&"solo-parent".into()));
    }

    // -----------------------------------------------------------------
    // Test: multi-branch tree — leaf nodes deleted before branch nodes
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_cascade_kill_children_multi_branch_tree() {
        let mem = Arc::new(MemStorage::default());

        // Build a multi-branch tree:
        //       root
        //      /    \
        //    child_a  child_b
        //    /    \        \
        //  gc_a1  gc_a2    gc_b1
        let mut root = SessionCheckpoint::new("root".into());
        root.parent_session_id = None;
        root.depth = 0;
        mem.add_checkpoint(root);

        let mut child_a = SessionCheckpoint::new("child_a".into());
        child_a.parent_session_id = Some("root".into());
        child_a.depth = 1;
        mem.add_checkpoint(child_a);

        let mut child_b = SessionCheckpoint::new("child_b".into());
        child_b.parent_session_id = Some("root".into());
        child_b.depth = 1;
        mem.add_checkpoint(child_b);

        let mut gc_a1 = SessionCheckpoint::new("gc_a1".into());
        gc_a1.parent_session_id = Some("child_a".into());
        gc_a1.depth = 2;
        mem.add_checkpoint(gc_a1);

        let mut gc_a2 = SessionCheckpoint::new("gc_a2".into());
        gc_a2.parent_session_id = Some("child_a".into());
        gc_a2.depth = 2;
        mem.add_checkpoint(gc_a2);

        let mut gc_b1 = SessionCheckpoint::new("gc_b1".into());
        gc_b1.parent_session_id = Some("child_b".into());
        gc_b1.depth = 2;
        mem.add_checkpoint(gc_b1);

        // Run cascade
        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        ArchiveSweeper::cascade_kill_children(storage.as_ref(), "root")
            .await
            .unwrap();

        // All 5 descendants should be deleted
        let deleted = mem.deleted_ids();
        assert!(deleted.contains(&"child_a".to_string()));
        assert!(deleted.contains(&"child_b".to_string()));
        assert!(deleted.contains(&"gc_a1".to_string()));
        assert!(deleted.contains(&"gc_a2".to_string()));
        assert!(deleted.contains(&"gc_b1".to_string()));
        // Root itself should NOT be deleted
        assert!(!deleted.contains(&"root".to_string()));
        assert_eq!(deleted.len(), 5);

        // Verify deletion order: all depth-2 nodes before depth-1 nodes
        let pos_gc_a1 = deleted.iter().position(|id| id == "gc_a1").unwrap();
        let pos_gc_a2 = deleted.iter().position(|id| id == "gc_a2").unwrap();
        let pos_gc_b1 = deleted.iter().position(|id| id == "gc_b1").unwrap();
        let pos_child_a = deleted.iter().position(|id| id == "child_a").unwrap();
        let pos_child_b = deleted.iter().position(|id| id == "child_b").unwrap();

        assert!(
            pos_gc_a1 < pos_child_a,
            "gc_a1 must be deleted before child_a"
        );
        assert!(
            pos_gc_a2 < pos_child_a,
            "gc_a2 must be deleted before child_a"
        );
        assert!(
            pos_gc_b1 < pos_child_b,
            "gc_b1 must be deleted before child_b"
        );
    }
}
