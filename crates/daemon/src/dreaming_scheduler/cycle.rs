//! Dreaming-cycle execution for the DreamingScheduler.
//!
//! Houses the standalone [`execute_dreaming_cycle`] (dreaming pipeline →
//! mining scan → forgetting cleanup, spawnable without borrowing the
//! scheduler), the shutdown grace wrapper [`wait_for_active_task`], and
//! the `impl DreamingScheduler` methods that drive or serve a single
//! cycle: [`DreamingScheduler::spawn_or_skip_cycle`],
//! [`DreamingScheduler::run_once`],
//! [`DreamingScheduler::handle_mining_notify`] and
//! [`DreamingScheduler::mine_session`].

use std::sync::Arc;
use std::time::Duration;

use closeclaw_config::session::SessionConfigProvider;
use closeclaw_memory::dreaming::DreamingPipeline;
use closeclaw_memory::miner::MemoryMiner;
use closeclaw_memory::storage::MemoryStorage;
use closeclaw_session::persistence::PersistenceService;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use super::mining::mine_archived_session;
use super::{DreamingScheduler, DreamingSchedulerError, SHUTDOWN_GRACE_SECS};

// ── Standalone helpers (spawnable without borrowing scheduler) ───────

/// Execute one dreaming cycle: dreaming pipeline first, then mining scan.
///
/// Standalone async function so it can be spawned via `tokio::spawn`
/// without borrowing the scheduler.
async fn execute_dreaming_cycle(
    storage: &Arc<dyn PersistenceService>,
    memory_storage: &Arc<dyn MemoryStorage>,
    config: &Arc<dyn SessionConfigProvider>,
    dreaming_pipeline: &Arc<DreamingPipeline>,
    memory_miner: &Arc<MemoryMiner>,
) -> Result<(), DreamingSchedulerError> {
    let agents = config.list_agents();
    if agents.is_empty() {
        return Ok(());
    }

    // Step 1: Run dreaming pipeline (process already-mined entries)
    if let Err(e) = dreaming_pipeline.run_once(memory_storage.as_ref()).await {
        error!(%e, "dreaming pipeline failed");
    }

    // Step 2: Run mining scan (extract entries from new archived sessions)
    let unmined = storage.list_archived_unmined_sessions().await?;
    for session_id in unmined {
        mine_archived_session(&session_id, &agents, storage, memory_storage, memory_miner).await;
    }

    // Step 3: Run forgetting cleanup (delete expired events + orphan entities)
    match memory_miner.run_forgetting_cleanup().await {
        Ok(stats) => {
            if stats.events_deleted > 0 || stats.entities_deleted > 0 {
                info!(
                    events_deleted = stats.events_deleted,
                    entities_deleted = stats.entities_deleted,
                    "forgetting cleanup stats"
                );
            }
        }
        Err(e) => {
            warn!(%e, "forgetting cleanup failed");
        }
    }

    Ok(())
}

/// Wait for an active dreaming task to complete, with a timeout.
///
/// Logs the outcome. Used during shutdown to give an in-progress
/// cycle a chance to finish gracefully.
pub(crate) async fn wait_for_active_task(handle: JoinHandle<Result<(), DreamingSchedulerError>>) {
    match tokio::time::timeout(Duration::from_secs(SHUTDOWN_GRACE_SECS), handle).await {
        Ok(Ok(Ok(()))) => {
            info!("dreaming cycle completed before shutdown");
        }
        Ok(Ok(Err(e))) => {
            error!(%e, "dreaming cycle error during shutdown");
        }
        Ok(Err(e)) => {
            error!(
                %e,
                "dreaming task panicked during shutdown"
            );
        }
        Err(_) => {
            warn!(
                "dreaming cycle timed out after {}s, \
                 forcing exit",
                SHUTDOWN_GRACE_SECS
            );
        }
    }
}

impl DreamingScheduler {
    /// Shared mining-notify branch: mine the notified session only when
    /// at least one agent is configured; otherwise stay silent.
    pub(crate) async fn handle_mining_notify(&self, session_id: &str) {
        let agents = self.config.list_agents();
        if !agents.is_empty() {
            info!(
                session_id = %session_id,
                "mining triggered by archive notification"
            );
            self.mine_session(session_id, &agents).await;
        }
    }

    /// Spawn a dreaming cycle if none is currently active.
    ///
    /// If a previous task has already finished, its result is
    /// collected and logged before a new cycle is started.
    pub(crate) async fn spawn_or_skip_cycle(
        &self,
        active_task: &mut Option<JoinHandle<Result<(), DreamingSchedulerError>>>,
    ) {
        // Collect result of finished task
        if let Some(ref handle) = active_task {
            if handle.is_finished() {
                let handle = active_task.take().unwrap();
                match handle.await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        error!(%e, "dreaming cycle error");
                    }
                    Err(e) => {
                        error!(%e, "dreaming task panicked");
                    }
                }
            }
        }

        // Spawn new task if none active
        if active_task.is_none() {
            let storage = self.storage.clone();
            let memory_storage = self.memory_storage.clone();
            let config = self.config.clone();
            let pipeline = self.dreaming_pipeline.clone();
            let miner = self.memory_miner.clone();
            *active_task = Some(tokio::spawn(async move {
                execute_dreaming_cycle(&storage, &memory_storage, &config, &pipeline, &miner).await
            }));
        }
    }

    /// Execute one cycle: dreaming first, then mining.
    pub async fn run_once(&self) -> Result<(), DreamingSchedulerError> {
        execute_dreaming_cycle(
            &self.storage,
            &self.memory_storage,
            &self.config,
            &self.dreaming_pipeline,
            &self.memory_miner,
        )
        .await
    }

    /// Mine a single archived session: load checkpoint, filter by agent,
    /// format transcript, and invoke the memory miner.
    async fn mine_session(&self, session_id: &str, agents: &[String]) {
        mine_archived_session(
            session_id,
            agents,
            &self.storage,
            &self.memory_storage,
            &self.memory_miner,
        )
        .await
    }
}
