//! Memory Miner — extract structured memory entries from session transcripts.
//!
//! Runs as an independent task. Input is a session transcript; output is a
//! set of structured [`MemoryEntry`] items written to the memory store, plus
//! a `mined=true` flag on the source session.

use thiserror::Error;

use crate::memory::dreaming::MemoryEntry;
use crate::session::persistence::{PersistenceError, PersistenceService};

/// Errors specific to the memory-miner.
#[derive(Debug, Error)]
pub enum MinerError {
    /// Storage layer error.
    #[error("storage error: {0}")]
    Storage(#[from] PersistenceError),

    /// An I/O error occurred while reading or writing memory files.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// The transcript could not be parsed.
    #[error("transcript parse error: {0}")]
    TranscriptParse(String),
}

/// Memory miner — extracts structured entries from session transcripts.
pub struct MemoryMiner;

impl MemoryMiner {
    /// Create a new `MemoryMiner`.
    pub fn new() -> Self {
        Self
    }

    /// Mine a single session: read transcript → extract → write → mark.
    pub async fn mine_session(
        &self,
        session_id: &str,
        storage: &dyn PersistenceService,
    ) -> Result<(), MinerError> {
        let checkpoint = storage.load_checkpoint(session_id).await?.ok_or_else(|| {
            MinerError::TranscriptParse(format!("session {session_id} not found"))
        })?;

        if checkpoint.mined {
            return Ok(());
        }

        let entries = self.extract_entries(session_id);
        self.write_entries(&entries, storage).await?;
        storage.mark_mined(session_id).await?;
        Ok(())
    }

    /// Extract memory-worthy entries from a session transcript.
    ///
    /// This is the core extraction logic. In a full implementation it would
    /// parse the transcript and apply heuristics. Here we provide the
    /// structural skeleton.
    fn extract_entries(&self, session_id: &str) -> Vec<MemoryEntry> {
        tracing::warn!(
            session_id,
            "extract_entries not yet implemented, returning empty"
        );
        Vec::new()
    }

    /// Persist extracted entries to the memory store.
    async fn write_entries(
        &self,
        _entries: &[MemoryEntry],
        _storage: &dyn PersistenceService,
    ) -> Result<(), MinerError> {
        tracing::warn!("write_entries not yet implemented, no-op");
        Ok(())
    }
}

impl Default for MemoryMiner {
    fn default() -> Self {
        Self::new()
    }
}
