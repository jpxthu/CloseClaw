//! Dreaming Scheduler — background task for memory promotion and mining.
//!
//! Periodically triggers the dreaming pipeline (three-stage memory promotion)
//! followed by memory-mining (session transcript extraction). Spawned by the
//! daemon at startup and shut down gracefully via a `tokio::sync::watch`
//! channel.
//!
//! Supports cron-based scheduling via `DreamingConfig.schedule`. When a valid
//! cron expression is provided, the scheduler computes wall-clock next-fire
//! times. On parse failure it falls back to a fixed interval from
//! `SessionConfigProvider::dreaming_interval_secs()`.

use std::sync::Arc;

use thiserror::Error;
use tokio::sync::mpsc;

use closeclaw_config::session::SessionConfigProvider;
use closeclaw_config::ConfigChangeEvent;
use closeclaw_config::ConfigManager;
use closeclaw_memory::dreaming::DreamingPipeline;
use closeclaw_memory::miner::MemoryMiner;
use closeclaw_memory::storage::MemoryStorage;
use closeclaw_session::persistence::{PersistenceError, PersistenceService};

/// Timeout (seconds) to wait for an active dreaming cycle during shutdown.
pub(crate) const SHUTDOWN_GRACE_SECS: u64 = 10;

mod config_reload;
mod cron;
mod cycle;
mod events;
mod mining;
mod run;

// Re-export so sibling modules and the crate-level test module keep
// using the `crate::dreaming_scheduler::wait_for_active_task` path.
pub(crate) use cycle::wait_for_active_task;

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
/// Follows the same background-task pattern as
/// [`ArchiveSweeper`](closeclaw_gateway::sweeper::ArchiveSweeper):
/// `tokio::spawn` + `watch::Receiver` for shutdown coordination.
pub struct DreamingScheduler {
    storage: Arc<dyn PersistenceService>,
    /// Memory-side narrow storage port (adapter over `storage`).
    memory_storage: Arc<dyn MemoryStorage>,
    config: Arc<dyn SessionConfigProvider>,
    dreaming_pipeline: Arc<DreamingPipeline>,
    memory_miner: Arc<MemoryMiner>,
    /// Optional cron expression from `DreamingConfig.schedule`.
    pub(crate) schedule: Option<String>,
    /// Config manager for hot-reload subscriptions.
    config_manager: Arc<ConfigManager>,
    /// Broadcast receiver for config change events.
    config_rx: tokio::sync::broadcast::Receiver<ConfigChangeEvent>,
    /// Optional channel receiver for immediate mining notifications
    /// from ArchiveSweeper (design doc §即时 hook).
    mining_notify_rx: Option<mpsc::Receiver<String>>,
}

/// Events shared by both scheduler loops: the four `select!` branches
/// (shutdown watch, config broadcast, mining-notify channel, timer)
/// collapsed into a single classification returned by
/// [`DreamingScheduler::next_event`].
enum LoopEvent {
    /// Shutdown signal observed — finish the active cycle, then exit.
    Shutdown,
    /// Memory-section config reload — each loop applies its own handler.
    ConfigReload,
    /// Session ID pushed by ArchiveSweeper's immediate hook.
    MiningNotify(String),
    /// Scheduled fire time reached — run one cycle, then advance
    /// `next_fire` per the loop's own strategy.
    Fire,
    /// Config broadcast channel closed (sender dropped) — finish the
    /// active cycle via the shutdown path, then exit (issue #3577).
    ConfigClosed,
}

/// Outcome of one receive from the config broadcast channel, classified
/// by [`classify_config_recv`](events::classify_config_recv).
enum ConfigRecvOutcome {
    /// Memory-section reload — the caller applies its reload handler.
    MemoryReload,
    /// Other section / non-reload event — ignore and keep polling.
    Ignore,
    /// Receiver lagged behind the broadcast: `n` events were lost.
    /// Log and keep polling.
    Lagged(u64),
    /// Sender half dropped — the channel is permanently closed and the
    /// loop must terminate via the shutdown path (issue #3577).
    Closed,
}

/// What triggered a loop's shutdown epilogue
/// ([`DreamingScheduler::finish_shutdown`]).
enum ShutdownCause {
    /// Shutdown watch signal observed (normal daemon shutdown).
    Signal,
    /// Config broadcast channel closed — sender dropped (issue #3577).
    ConfigClosed,
}

impl DreamingScheduler {
    /// Create a new `DreamingScheduler`.
    pub fn new(
        storage: Arc<dyn PersistenceService>,
        memory_storage: Arc<dyn MemoryStorage>,
        config: Arc<dyn SessionConfigProvider>,
        dreaming_pipeline: Arc<DreamingPipeline>,
        memory_miner: Arc<MemoryMiner>,
        config_manager: Arc<ConfigManager>,
    ) -> Self {
        let config_rx = config_manager.subscribe_config_changes();
        Self {
            storage,
            memory_storage,
            config,
            dreaming_pipeline,
            memory_miner,
            schedule: None,
            config_manager,
            config_rx,
            mining_notify_rx: None,
        }
    }

    /// Set the cron schedule expression for dreaming.
    pub fn with_schedule(mut self, schedule: Option<String>) -> Self {
        self.schedule = schedule;
        self
    }

    /// Attach the mining notify channel receiver.  When a session
    /// is archived by the [`ArchiveSweeper`](closeclaw_gateway::sweeper::ArchiveSweeper),
    /// the sender emits the session ID through this channel so mining
    /// starts immediately.
    pub fn with_mining_notify_rx(mut self, rx: mpsc::Receiver<String>) -> Self {
        self.mining_notify_rx = Some(rx);
        self
    }
}

impl std::fmt::Debug for DreamingScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DreamingScheduler")
            .field("schedule", &self.schedule)
            .field("config_manager", &"<ConfigManager>")
            .finish()
    }
}
