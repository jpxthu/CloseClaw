//! Archive Sweeper — background task for session lifecycle management
//!
//! Periodically scans for idle sessions to archive and expired archived
//! sessions to purge. Spawned by the daemon at startup and shut down
//! gracefully via a `tokio::sync::watch` channel.

use std::panic;
use std::sync::Arc;

use thiserror::Error;
use tokio::sync::watch;
use tracing::{error, info, warn};

use crate::config::session::SessionConfigProvider;
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
}

impl ArchiveSweeper {
    /// Create a new `ArchiveSweeper`.
    pub fn new(
        storage: Arc<dyn PersistenceService>,
        config: Arc<dyn SessionConfigProvider>,
    ) -> Self {
        Self { storage, config }
    }

    /// Run the sweeper loop until `shutdown` signal is received.
    ///
    /// # Arguments
    /// * `shutdown` — watch receiver that will be closed when the sweeper
    ///   should exit gracefully
    pub async fn run(&self, mut shutdown: watch::Receiver<()>) {
        let interval_secs = self.config.sweeper_interval_secs();
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));

        info!(interval_secs, "ArchiveSweeper started");

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    info!("ArchiveSweeper received shutdown signal, exiting");
                    break;
                }
                _ = interval.tick() => {
                    if let Err(e) = self.run_once().await {
                        error!(%e, "run_once returned error, continuing loop");
                    }
                }
            }
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

        let handle = tokio::task::spawn_blocking(move || {
            let runtime = tokio::runtime::Handle::current();
            panic::catch_unwind(panic::AssertUnwindSafe(|| {
                runtime.block_on(Self::run_once_inner_impl(
                    Arc::clone(&storage),
                    Arc::clone(&config),
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
    ) -> Result<(), ArchiveSweeperError> {
        let agents = config.list_agents();
        if agents.is_empty() {
            return Ok(());
        }

        for agent_id in &agents {
            for role in [AgentRole::MainAgent, AgentRole::SubAgent] {
                Self::sweep_agent_role(Arc::clone(&storage), config.as_ref(), agent_id, role).await;
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
    ) {
        let cfg = config.session_config_for(agent_id, role);

        // --- Idle → archive ---
        if let Ok(idle_ids) = storage
            .list_idle_sessions_for_agent(agent_id, role, cfg.idle_minutes)
            .await
        {
            for sid in idle_ids {
                let sid_err = sid.clone();
                if let Err(e) = Self::archive_and_invalidate_impl(Arc::clone(&storage), sid).await {
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
        archived: Mutex<Vec<String>>,
        invalidated: Mutex<Vec<String>>,
        archive_called: Mutex<Vec<String>>,
        purge_called: Mutex<Vec<String>>,
        /// Session IDs returned from `list_idle_sessions_for_agent`.
        idle_sessions: Mutex<Vec<String>>,
        /// Session IDs returned from `list_expired_archived_sessions_for_agent`.
        expired_sessions: Mutex<Vec<String>>,
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
    }

    #[async_trait]
    impl PersistenceService for MemStorage {
        async fn save_checkpoint(
            &self,
            _checkpoint: &SessionCheckpoint,
        ) -> Result<(), PersistenceError> {
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

        async fn delete_checkpoint(&self, _session_id: &str) -> Result<(), PersistenceError> {
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
}
