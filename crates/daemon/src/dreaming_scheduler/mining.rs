//! Session-mining helpers for the DreamingScheduler.
//!
//! Free functions shared by the standalone dreaming cycle and the
//! notify-driven mining path: [`mine_archived_session`] mines one
//! archived session from its checkpoint (agent filtering included),
//! and [`format_transcript`] renders pending messages into the raw
//! transcript text expected by the memory miner.

use std::sync::Arc;

use closeclaw_memory::miner::MemoryMiner;
use closeclaw_memory::storage::MemoryStorage;
use closeclaw_session::persistence::PersistenceService;
use tracing::error;

/// Mine a single archived session (standalone version).
pub(crate) async fn mine_archived_session(
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

/// Format pending messages into the raw transcript text expected by the
/// miner.
///
/// Messages are rendered as `"<role>: <content>"` lines, matching the
/// format produced by session transcript recording.
pub(crate) fn format_transcript(
    messages: &[closeclaw_session::persistence::PendingMessage],
) -> String {
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
