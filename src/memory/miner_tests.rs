//! Additional unit tests for MemoryMiner.
//!
//! Complements the inline tests in miner.rs with additional edge cases.

use std::sync::Mutex;

use async_trait::async_trait;

use crate::memory::miner::MemoryMiner;
use crate::session::persistence::{PersistenceError, PersistenceService, SessionCheckpoint};

// ── Test helpers ─────────────────────────────────────────────────────────

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

// ── Tests ────────────────────────────────────────────────────────────────

/// Mining a session with a transcript marks it as mined.
#[tokio::test]
async fn test_mine_session_marks_mined_with_transcript() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-transcript".into());
    cp.mined = false;
    storage.add_checkpoint(cp);

    let miner = MemoryMiner::new();
    miner
        .mine_session("sess-transcript", &storage)
        .await
        .unwrap();

    let mined = storage.mined_ids();
    assert!(
        mined.contains(&"sess-transcript".to_string()),
        "session should be marked as mined"
    );
}

/// Mining an already-mined session is a no-op (idempotent).
#[tokio::test]
async fn test_mine_session_idempotent_on_already_mined() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-idempotent".into());
    cp.mined = true;
    storage.add_checkpoint(cp);

    let miner = MemoryMiner::new();
    miner
        .mine_session("sess-idempotent", &storage)
        .await
        .unwrap();

    let mined = storage.mined_ids();
    assert!(
        mined.is_empty(),
        "should not call mark_mined again for already-mined session"
    );
}

/// Mining a nonexistent session returns an error.
#[tokio::test]
async fn test_mine_session_nonexistent_returns_error() {
    let storage = TestStorage::default();
    let miner = MemoryMiner::new();
    let result = miner.mine_session("does-not-exist", &storage).await;
    assert!(result.is_err(), "mining nonexistent session should fail");
}

/// Mining multiple sessions processes each independently.
#[tokio::test]
async fn test_mine_multiple_sessions() {
    let storage = TestStorage::default();
    for i in 0..5 {
        let cp = SessionCheckpoint::new(format!("sess-{i}"));
        storage.add_checkpoint(cp);
    }

    let miner = MemoryMiner::new();
    for i in 0..5 {
        miner
            .mine_session(&format!("sess-{i}"), &storage)
            .await
            .unwrap();
    }

    let mined = storage.mined_ids();
    assert_eq!(mined.len(), 5, "all 5 sessions should be mined");
    for i in 0..5 {
        assert!(
            mined.contains(&format!("sess-{i}")),
            "sess-{i} should be in mined list"
        );
    }
}
