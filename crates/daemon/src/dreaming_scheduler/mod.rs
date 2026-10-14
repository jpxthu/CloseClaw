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
use std::time::Duration;

use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

/// Timeout (seconds) to wait for an active dreaming cycle during shutdown.
pub(crate) const SHUTDOWN_GRACE_SECS: u64 = 10;

use closeclaw_config::session::SessionConfigProvider;
use closeclaw_config::ConfigChangeEvent;
use closeclaw_config::ConfigManager;
use closeclaw_memory::dreaming::DreamingPipeline;
use closeclaw_memory::miner::MemoryMiner;
use closeclaw_memory::storage::MemoryStorage;
use closeclaw_session::persistence::{PersistenceError, PersistenceService};

mod config_reload;
mod cron;
mod events;
mod run;

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
    pub(crate) config_rx: tokio::sync::broadcast::Receiver<ConfigChangeEvent>,
    /// Optional channel receiver for immediate mining notifications
    /// from ArchiveSweeper (design doc §即时 hook).
    pub(crate) mining_notify_rx: Option<mpsc::Receiver<String>>,
}

/// Events shared by both scheduler loops: the four `select!` branches
/// (shutdown watch, config broadcast, mining-notify channel, timer)
/// collapsed into a single classification returned by
/// [`DreamingScheduler::next_event`].
pub(crate) enum LoopEvent {
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
pub(crate) enum ConfigRecvOutcome {
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
pub(crate) enum ShutdownCause {
    /// Shutdown watch signal observed (normal daemon shutdown).
    Signal,
    /// Config broadcast channel closed — sender dropped (issue #3577).
    ConfigClosed,
}

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

/// Mine a single archived session (standalone version).
async fn mine_archived_session(
    session_id: &str,
    agents: &[String],
    storage: &Arc<dyn PersistenceService>,
    memory_storage: &Arc<dyn MemoryStorage>,
    memory_miner: &Arc<MemoryMiner>,
) {
    let checkpoint = match storage.load_archived_checkpoint(session_id).await {
        Ok(Some(cp)) => cp,
        Ok(None) => {
            error!(
                session_id = %session_id,
                "archived checkpoint not found, skipping"
            );
            return;
        }
        Err(e) => {
            error!(
                session_id = %session_id,
                %e,
                "failed to load archived checkpoint"
            );
            return;
        }
    };

    if let Some(ref aid) = checkpoint.agent_id {
        if !agents.contains(aid) {
            return;
        }
    }

    let raw_transcript = format_transcript(&checkpoint.outbound_pending);
    let agent_id = checkpoint.agent_id.as_deref().unwrap_or("");

    if let Err(e) = memory_miner
        .mine_session_from_checkpoint(
            session_id,
            &raw_transcript,
            agent_id,
            &crate::memory_storage_adapter::checkpoint_snapshot(&checkpoint),
            memory_storage.as_ref(),
        )
        .await
    {
        error!(
            session_id = %session_id,
            %e,
            "failed to mine session"
        );
    }
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

    /// Attach the mining notify channel receiver.  When a session is
    /// archived by the [`ArchiveSweeper`], the sender emits the
    /// session ID through this channel so mining starts immediately.
    pub fn with_mining_notify_rx(mut self, rx: mpsc::Receiver<String>) -> Self {
        self.mining_notify_rx = Some(rx);
        self
    }

    /// Shared mining-notify branch: mine the notified session only when
    /// at least one agent is configured; otherwise stay silent.
    async fn handle_mining_notify(&self, session_id: &str) {
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
    async fn spawn_or_skip_cycle(
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

/// Format pending messages into the raw transcript text expected by the
/// miner.
///
/// Messages are rendered as `"<role>: <content>"` lines, matching the
/// format produced by session transcript recording.
fn format_transcript(messages: &[closeclaw_session::persistence::PendingMessage]) -> String {
    messages
        .iter()
        .map(|m| {
            let role = m
                .role
                .as_deref()
                .filter(|r| !r.is_empty())
                .unwrap_or("unknown");
            format!("{role}: {}", m.content)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl std::fmt::Debug for DreamingScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DreamingScheduler")
            .field("schedule", &self.schedule)
            .field("config_manager", &"<ConfigManager>")
            .finish()
    }
}

// ── Transcript format tests ───────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_transcript_empty() {
        let result = format_transcript(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_format_transcript_converts_messages() {
        use closeclaw_session::persistence::PendingMessage;
        let messages = vec![
            PendingMessage::with_role("msg1".into(), "hello".into(), "user".into()),
            PendingMessage::with_role("msg2".into(), "hi there".into(), "assistant".into()),
        ];
        let result = format_transcript(&messages);
        assert!(result.contains("user: hello"));
        assert!(result.contains("assistant: hi there"));
    }

    #[test]
    fn test_format_transcript_handles_empty_role() {
        use closeclaw_session::persistence::PendingMessage;
        let messages = vec![PendingMessage::new("msg1".into(), "content".into())];
        let result = format_transcript(&messages);
        assert!(result.contains("unknown: content"));
    }

    #[test]
    fn test_format_transcript_uses_role_not_message_id() {
        use closeclaw_session::persistence::PendingMessage;
        let messages = vec![
            PendingMessage::with_role("out-12345".into(), "hello".into(), "assistant".into()),
            PendingMessage::with_role("pending-67890".into(), "world".into(), "user".into()),
        ];
        let result = format_transcript(&messages);
        assert!(result.contains("assistant: hello"));
        assert!(result.contains("user: world"));
        assert!(!result.contains("out-12345"));
        assert!(!result.contains("pending-67890"));
    }
}
