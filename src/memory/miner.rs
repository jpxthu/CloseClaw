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
        let _ = session_id;
        // TODO: parse transcript and extract durable entries.
        Vec::new()
    }

    /// Persist extracted entries to the memory store.
    async fn write_entries(
        &self,
        entries: &[MemoryEntry],
        storage: &dyn PersistenceService,
    ) -> Result<(), MinerError> {
        // TODO: write to Markdown files in the memory store.
        let _ = (entries, storage);
        Ok(())
    }
}

impl Default for MemoryMiner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::persistence::{PersistenceError, SessionCheckpoint};
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// Minimal in-memory storage for miner tests.
    #[derive(Debug, Default)]
    struct TestStorage {
        checkpoints: Mutex<Vec<SessionCheckpoint>>,
        mined_ids: Mutex<Vec<String>>,
    }

    impl TestStorage {
        fn add_checkpoint(&self, cp: SessionCheckpoint) {
            self.checkpoints.lock().unwrap().push(cp);
        }

        fn mined_ids(&self) -> Vec<String> {
            self.mined_ids.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl PersistenceService for TestStorage {
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
            Ok(self
                .checkpoints
                .lock()
                .unwrap()
                .iter()
                .find(|cp| cp.session_id == session_id)
                .cloned())
        }

        async fn delete_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError> {
            self.checkpoints
                .lock()
                .unwrap()
                .retain(|cp| cp.session_id != session_id);
            Ok(())
        }

        async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }

        async fn mark_mined(&self, session_id: &str) -> Result<(), PersistenceError> {
            self.mined_ids.lock().unwrap().push(session_id.into());
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_mine_session_marks_mined() {
        let storage = TestStorage::default();
        let cp = SessionCheckpoint::new("sess-1".into());
        storage.add_checkpoint(cp);

        let miner = MemoryMiner::new();
        miner.mine_session("sess-1", &storage).await.unwrap();

        let mined = storage.mined_ids();
        assert!(mined.contains(&"sess-1".to_string()));
    }

    #[tokio::test]
    async fn test_mine_session_skips_already_mined() {
        let storage = TestStorage::default();
        let mut cp = SessionCheckpoint::new("sess-2".into());
        cp.mined = true;
        storage.add_checkpoint(cp);

        let miner = MemoryMiner::new();
        miner.mine_session("sess-2", &storage).await.unwrap();

        let mined = storage.mined_ids();
        assert!(mined.is_empty(), "should not re-mine");
    }

    #[tokio::test]
    async fn test_mine_session_not_found_returns_error() {
        let storage = TestStorage::default();
        let miner = MemoryMiner::new();
        let result = miner.mine_session("nonexistent", &storage).await;
        assert!(result.is_err());
    }
}
