//! Dreaming Scheduler — background task for memory promotion and mining.
//!
//! Periodically triggers the dreaming pipeline (three-stage memory promotion)
//! followed by memory-mining (session transcript extraction). Spawned by the
//! daemon at startup and shut down gracefully via a `tokio::sync::watch`
//! channel.

use std::sync::Arc;

use thiserror::Error;
use tokio::sync::watch;
use tokio::time::Instant;
use tracing::{error, info};

use crate::config::session::SessionConfigProvider;
use crate::memory::dreaming::DreamingPipeline;
use crate::memory::miner::MemoryMiner;
use crate::session::persistence::{PersistenceError, PersistenceService};

/// Errors that can occur during scheduler operations.
#[derive(Debug, Error)]
pub enum DreamingSchedulerError {
    /// Storage layer error.
    #[error("storage error: {0}")]
    Storage(#[from] PersistenceError),

    /// Dreaming pipeline error.
    #[error("dreaming error: {0}")]
    Dreaming(String),

    /// Memory miner error.
    #[error("miner error: {0}")]
    Miner(String),
}

/// Dreaming Scheduler — orchestrates dreaming pipeline and memory mining.
///
/// Follows the same background-task pattern as [`ArchiveSweeper`]:
/// `tokio::spawn` + `watch::Receiver` for shutdown coordination.
pub struct DreamingScheduler {
    storage: Arc<dyn PersistenceService>,
    config: Arc<dyn SessionConfigProvider>,
    dreaming_pipeline: Arc<DreamingPipeline>,
    memory_miner: Arc<MemoryMiner>,
}

impl DreamingScheduler {
    /// Create a new `DreamingScheduler`.
    pub fn new(
        storage: Arc<dyn PersistenceService>,
        config: Arc<dyn SessionConfigProvider>,
        dreaming_pipeline: Arc<DreamingPipeline>,
        memory_miner: Arc<MemoryMiner>,
    ) -> Self {
        Self {
            storage,
            config,
            dreaming_pipeline,
            memory_miner,
        }
    }

    /// Run the scheduler loop until `shutdown` signal is received.
    ///
    /// Each cycle: first dreaming pipeline, then mining scan.
    /// Follows the `select!` pattern from [`ArchiveSweeper`].
    pub async fn run(&self, mut shutdown: watch::Receiver<()>) {
        let interval_secs = self.config.dreaming_interval_secs();
        let interval = tokio::time::Duration::from_secs(interval_secs);
        let mut next_fire = Instant::now() + interval;

        info!("DreamingScheduler started with interval {}s", interval_secs);

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    info!("DreamingScheduler received shutdown signal, exiting");
                    break;
                }
                _ = tokio::time::sleep_until(next_fire) => {
                    if let Err(e) = self.run_once().await {
                        error!(%e, "DreamingScheduler run_once returned error, continuing loop");
                    }
                    next_fire += interval;
                    // Catch up if we fell behind
                    if Instant::now() > next_fire + interval {
                        next_fire = Instant::now() + interval;
                    }
                }
            }
        }
    }

    /// Execute one cycle: dreaming first, then mining.
    pub async fn run_once(&self) -> Result<(), DreamingSchedulerError> {
        let agents = self.config.list_agents();
        if agents.is_empty() {
            return Ok(());
        }

        // Step 1: Run dreaming pipeline (process already-mined entries)
        if let Err(e) = self.dreaming_pipeline.run_once(self.storage.as_ref()).await {
            error!(%e, "dreaming pipeline failed");
            // Continue to mining even if dreaming fails
        }

        // Step 2: Run mining scan (extract entries from new archived sessions)
        let unmined = self.storage.list_archived_unmined_sessions().await?;

        for session_id in unmined {
            // Look up agent_id from archived checkpoint to filter by configured agents
            let checkpoint = self.storage.load_archived_checkpoint(&session_id).await;
            let agent_id = match checkpoint {
                Ok(Some(cp)) => cp.agent_id,
                _ => None,
            };

            // Skip sessions whose agent is not in the configured agent list
            if let Some(ref aid) = agent_id {
                if !agents.contains(aid) {
                    continue;
                }
            }

            if let Err(e) = self
                .memory_miner
                .mine_session(&session_id, self.storage.as_ref())
                .await
            {
                error!(session_id = %session_id, %e, "failed to mine session");
            }
        }

        Ok(())
    }
}

impl std::fmt::Debug for DreamingScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DreamingScheduler").finish()
    }
}
