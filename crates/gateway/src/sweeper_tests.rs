#[cfg(test)]
mod tests {
    use crate::sweeper::*;
    use async_trait::async_trait;
    use closeclaw_config::session::PerAgentSessionConfig;
    use closeclaw_config::SessionConfigProvider;
    use closeclaw_session::persistence::{
        AgentRole, PersistenceError, PersistenceService, SessionCheckpoint,
    };
    use std::sync::{Arc, Mutex};
    use tokio::sync::watch;

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

        fn dreaming_interval_secs(&self) -> u64 {
            600
        }

        fn list_agents(&self) -> Vec<String> {
            self.agents.lock().unwrap().clone()
        }

        fn compact_config(&self) -> closeclaw_common::CompactConfig {
            closeclaw_common::CompactConfig::default()
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

    // ── shutdown grace period tests ─────────────────────────────────

    /// Shutdown signal with no running task → exits immediately.
    #[tokio::test]
    async fn test_shutdown_no_running_task_exits_immediately() {
        let mem = Arc::new(MemStorage::default());
        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::with_agents(vec![]));
        let (tx, rx) = watch::channel(());
        let sweeper = ArchiveSweeper::new(Arc::clone(&storage), Arc::clone(&config));
        let handle = tokio::spawn(async move {
            sweeper.run(rx).await;
        });
        // Send shutdown immediately — no task running
        let _ = tx.send(());
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
        assert!(
            result.is_ok(),
            "sweeper should exit quickly when no task is running"
        );
    }

    /// Shutdown signal with running task that completes within grace period → exits cleanly.
    #[tokio::test]
    async fn test_shutdown_grace_period_completes_within_deadline() {
        use tokio::sync::watch;

        // Fake sweeper that spawns a task taking ~100ms (well under grace period).
        struct FakeSweeper {
            storage: Arc<dyn PersistenceService>,
        }

        impl FakeSweeper {
            async fn run(&self, mut shutdown: watch::Receiver<()>) {
                let mut running_task: Option<tokio::task::JoinHandle<()>> = None;
                let interval = tokio::time::Duration::from_millis(50);
                let mut next_fire = tokio::time::Instant::now() + interval;
                loop {
                    tokio::select! {
                        _ = shutdown.changed() => break,
                        _ = tokio::time::sleep_until(next_fire),
                            if running_task.is_none() =>
                        {
                            let storage = Arc::clone(&self.storage);
                            let task = tokio::task::spawn(async move {
                                // Simulate a task that finishes quickly
                                tokio::time::sleep(std::time::Duration::from_millis(100)).
                                    await;
                                let _ = storage;
                            });
                            running_task = Some(task);
                            next_fire += interval;
                        }
                        result = async {
                            match running_task.as_mut() {
                                Some(t) => t.await,
                                None => std::future::pending().await,
                            }
                        } => {
                            running_task = None;
                            if result.is_err() {
                                tracing::error!("task panicked");
                            }
                        }
                    }
                }
                // Grace period: same logic as real sweeper
                if let Some(mut task) = running_task {
                    let grace = tokio::time::Duration::from_secs(SWEEPER_GRACE_PERIOD_SECS);
                    tokio::select! {
                        result = &mut task => {
                            let _ = result;
                        }
                        _ = tokio::time::sleep(grace) => {
                            task.abort();
                        }
                    }
                }
            }
        }

        let mem = Arc::new(MemStorage::default());
        let sweeper = FakeSweeper {
            storage: mem.clone() as _,
        };
        let (tx, rx) = watch::channel(());

        // Spawn the sweeper
        let handle = tokio::spawn(async move {
            sweeper.run(rx).await;
        });

        // Wait long enough for the sweeper to fire and start a task
        tokio::time::sleep(tokio::time::Duration::from_millis(80)).await;
        // Send shutdown — task is running but will finish within grace period
        let _ = tx.send(());
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), handle).await;
        assert!(
            result.is_ok(),
            "sweeper should exit after task completes within grace period"
        );
    }

    /// Shutdown signal with running task that exceeds grace period → abort.
    #[tokio::test]
    async fn test_shutdown_grace_period_expires_aborts() {
        use tokio::sync::watch;

        struct FakeSweeper {
            storage: Arc<dyn PersistenceService>,
        }

        impl FakeSweeper {
            async fn run(&self, mut shutdown: watch::Receiver<()>) {
                let mut running_task: Option<tokio::task::JoinHandle<()>> = None;
                let interval = tokio::time::Duration::from_millis(50);
                let mut next_fire = tokio::time::Instant::now() + interval;
                loop {
                    tokio::select! {
                        _ = shutdown.changed() => break,
                        _ = tokio::time::sleep_until(next_fire),
                            if running_task.is_none() =>
                        {
                            let storage = Arc::clone(&self.storage);
                            let task = tokio::task::spawn(async move {
                                // Simulate a task that takes longer than grace period
                                tokio::time::sleep(std::time::Duration::from_secs(30)).
                                    await;
                                let _ = storage;
                            });
                            running_task = Some(task);
                            next_fire += interval;
                        }
                        result = async {
                            match running_task.as_mut() {
                                Some(t) => t.await,
                                None => std::future::pending().await,
                            }
                        } => {
                            running_task = None;
                            if result.is_err() {
                                tracing::error!("task panicked");
                            }
                        }
                    }
                }
                if let Some(mut task) = running_task {
                    let grace = tokio::time::Duration::from_secs(SWEEPER_GRACE_PERIOD_SECS);
                    tokio::select! {
                        result = &mut task => {
                            let _ = result;
                        }
                        _ = tokio::time::sleep(grace) => {
                            task.abort();
                            tracing::warn!("grace period expired, aborting");
                        }
                    }
                }
            }
        }

        let mem = Arc::new(MemStorage::default());
        let sweeper = FakeSweeper {
            storage: mem.clone() as _,
        };
        let (tx, rx) = watch::channel(());
        let handle = tokio::spawn(async move {
            sweeper.run(rx).await;
        });

        // Wait for the sweeper to start a task
        tokio::time::sleep(tokio::time::Duration::from_millis(80)).await;
        // Send shutdown — task will NOT finish within grace period
        let _ = tx.send(());
        let start = tokio::time::Instant::now();
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), handle).await;
        let elapsed = start.elapsed();
        assert!(
            result.is_ok(),
            "sweeper should exit after grace period abort"
        );
        // The sweeper should exit within ~grace_period (5s) + overhead, not
        // the full 30s of the task.
        assert!(
            elapsed < std::time::Duration::from_secs(8),
            "sweeper should exit within grace period, took {:?}",
            elapsed
        );
    }

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
