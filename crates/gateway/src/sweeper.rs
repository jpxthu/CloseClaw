//! Archive Sweeper — background task for session lifecycle management
//!
//! Periodically scans for idle sessions to archive and expired archived
//! sessions to purge. Spawned by the daemon at startup and shut down
//! gracefully via a `tokio::sync::watch` channel.

use std::panic;
use std::sync::Arc;

use thiserror::Error;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use tracing::{error, info, warn};

/// Grace period (in seconds) to wait for a running sweep to finish
/// before forcibly aborting it on shutdown.
pub(crate) const SWEEPER_GRACE_PERIOD_SECS: u64 = 10;

use closeclaw_config::session::SessionConfigProvider;
use closeclaw_session::persistence::{AgentRole, PersistenceError, PersistenceService};

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
    /// Channel sender for notifying the DreamingScheduler about archived
    /// sessions, enabling immediate mining (design doc §即时 hook).
    mining_notify_tx: Option<mpsc::Sender<String>>,
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
            mining_notify_tx: None,
        }
    }

    /// Attach a mining notify channel sender.  When a session is
    /// archived, the sender emits the session ID so the
    /// [`DreamingScheduler`] can trigger mining immediately.
    pub fn with_mining_notify_tx(mut self, tx: mpsc::Sender<String>) -> Self {
        self.mining_notify_tx = Some(tx);
        self
    }

    /// Run the sweeper loop until `shutdown` signal is received.
    ///
    /// When shutdown arrives, if a sweep is in progress the sweeper
    /// waits up to [`SWEEPER_GRACE_PERIOD_SECS`] for it to finish
    /// before forcibly aborting the task.
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
        let mut running_task: Option<tokio::task::JoinHandle<()>> = None;

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    info!("ArchiveSweeper received shutdown signal, exiting");
                    break;
                }
                _ = tokio::time::sleep_until(next_fire), if running_task.is_none() => {
                    let sweeper = Self {
                        storage: Arc::clone(&self.storage),
                        config: Arc::clone(&self.config),
                        mining_notify_tx: self.mining_notify_tx.clone(),
                    };
                    let task = tokio::task::spawn(async move {
                        let notify_tx = sweeper.mining_notify_tx.clone();
                        if let Err(e) = sweeper.run_once_with_notify(notify_tx).await {
                            error!(%e, "run_once returned error");
                        }
                    });
                    running_task = Some(task);
                    next_fire += interval;
                    if Instant::now() > next_fire + interval {
                        next_fire = Instant::now() + interval;
                    }
                }
                result = async {
                    match running_task.as_mut() {
                        Some(t) => t.await,
                        None => std::future::pending().await,
                    }
                } => {
                    running_task = None;
                    if let Err(e) = result {
                        error!(%e, "run_once task panicked, continuing");
                    }
                }
            }
        }

        // Grace period: if a sweep is still running, wait then abort
        Self::wait_grace_period(running_task).await;
    }

    /// Wait up to [`SWEEPER_GRACE_PERIOD_SECS`] for a running sweep to
    /// finish, then abort it if it does not complete in time.
    async fn wait_grace_period(task: Option<tokio::task::JoinHandle<()>>) {
        let Some(mut task) = task else {
            return;
        };
        let grace = tokio::time::Duration::from_secs(SWEEPER_GRACE_PERIOD_SECS);
        info!(
            secs = SWEEPER_GRACE_PERIOD_SECS,
            "ArchiveSweeper waiting for running sweep to finish before shutdown"
        );
        tokio::select! {
            result = &mut task => {
                match result {
                    Ok(()) => {
                        info!("Running sweep completed within grace period");
                    }
                    Err(e) => {
                        error!(%e, "Run-once task panicked during graceful shutdown");
                    }
                }
            }
            _ = tokio::time::sleep(grace) => {
                task.abort();
                warn!(
                    secs = SWEEPER_GRACE_PERIOD_SECS,
                    "Grace period expired, aborting running sweep"
                );
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
        self.run_once_with_notify(None).await
    }

    /// Execute one sweep, optionally forwarding mining notifications
    /// via `notify_tx`.
    async fn run_once_with_notify(
        &self,
        notify_tx: Option<mpsc::Sender<String>>,
    ) -> Result<(), ArchiveSweeperError> {
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
                    notify_tx,
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
        notify_tx: Option<mpsc::Sender<String>>,
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
                    notify_tx.as_ref(),
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
        notify_tx: Option<&mpsc::Sender<String>>,
    ) {
        let cfg = config.session_config_for(agent_id, role);

        // --- Idle → archive (cascade children first) ---
        if let Ok(idle_ids) = storage
            .list_idle_sessions_for_agent(agent_id, role, cfg.idle_minutes)
            .await
        {
            for sid in idle_ids {
                let sid_err = sid.clone();

                // Skip archiving if the session has pending operations
                // (design doc §Sweeper 机制: "pending_operations 为空").
                match storage.load_checkpoint(&sid).await {
                    Ok(Some(checkpoint)) if !checkpoint.pending_operations.is_empty() => {
                        warn!(
                            session_id = %sid_err,
                            pending_count = checkpoint.pending_operations.len(),
                            "skipping archive: session has pending operations"
                        );
                        continue;
                    }
                    Ok(_) => { /* pending_operations empty, proceed */ }
                    Err(e) => {
                        error!(
                            session_id = %sid_err,
                            %e,
                            "failed to load checkpoint for pending_operations check, skipping"
                        );
                        continue;
                    }
                }

                if let Err(e) = Self::cascade_archive_impl(Arc::clone(&storage), sid.clone()).await
                {
                    error!(session_id = %sid_err, %e, "failed to archive idle session");
                } else if let Some(tx) = notify_tx {
                    // Notify DreamingScheduler for immediate mining
                    if let Err(e) = tx.try_send(sid) {
                        warn!(session_id = %sid_err, %e, "failed to send mining notification");
                    }
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
    pub(crate) async fn archive_and_invalidate_impl(
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
    /// Sweeper operates only on the persistence layer. Runtime tracking
    /// tables are cleaned up lazily by SessionManager on next lookup
    /// (design doc §Sweeper 机制: passive self-healing).
    pub(crate) async fn cascade_archive_impl(
        storage: Arc<dyn PersistenceService>,
        session_id: String,
    ) -> Result<(), ArchiveSweeperError> {
        // Recursively kill all descendants (deepest → shallowest)
        Self::cascade_kill_children(storage.as_ref(), &session_id).await?;

        // Now archive the parent session itself
        Self::archive_and_invalidate_impl(Arc::clone(&storage), session_id.clone()).await?;
        info!(%session_id, "session archived after cascade cleanup");
        Ok(())
    }

    /// 删除 `parent_session_id` 的所有后代 session（先深后浅）。
    /// 使用 BFS 收集所有后代及其深度，按深度降序删除（叶节点优先）。
    pub(crate) async fn cascade_kill_children(
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
    pub(crate) async fn purge_and_invalidate_impl(
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
