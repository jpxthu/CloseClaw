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

use closeclaw_config::session::SessionConfigProvider;
use closeclaw_memory::dreaming::DreamingPipeline;
use closeclaw_memory::miner::MemoryMiner;
use closeclaw_session::persistence::{PersistenceError, PersistenceService};

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
            self.mine_session(&session_id, &agents).await;
        }

        Ok(())
    }

    /// Mine a single archived session: load checkpoint, filter by agent,
    /// format transcript, and invoke the memory miner.
    async fn mine_session(&self, session_id: &str, agents: &[String]) {
        let checkpoint = match self.storage.load_archived_checkpoint(session_id).await {
            Ok(Some(cp)) => cp,
            Ok(None) => {
                error!(session_id = %session_id, "archived checkpoint not found, skipping");
                return;
            }
            Err(e) => {
                error!(session_id = %session_id, %e, "failed to load archived checkpoint");
                return;
            }
        };

        if let Some(ref aid) = checkpoint.agent_id {
            if !agents.contains(aid) {
                return;
            }
        }

        let raw_transcript = format_transcript(&checkpoint.pending_messages);

        if let Err(e) = self
            .memory_miner
            .mine_session_from_checkpoint(
                session_id,
                &raw_transcript,
                &checkpoint,
                self.storage.as_ref(),
            )
            .await
        {
            error!(session_id = %session_id, %e, "failed to mine session");
        }
    }
}

/// Format pending messages into the raw transcript text expected by the miner.
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
        f.debug_struct("DreamingScheduler").finish()
    }
}

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
