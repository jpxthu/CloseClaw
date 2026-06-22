//! Additional unit tests for DreamingPipeline.
//!
//! Complements the inline tests in dreaming.rs with tests that require
//! mock PersistenceService interactions.

use std::sync::Mutex;

use async_trait::async_trait;

use crate::memory::dreaming::DreamingPipeline;
use crate::session::persistence::{
    DreamingStatus, PersistenceError, PersistenceService, SessionCheckpoint,
};

// ── Test helpers ─────────────────────────────────────────────────────────

/// Minimal in-memory storage for dreaming pipeline tests.
#[derive(Debug, Default)]
struct TestStorage {
    checkpoints: Mutex<Vec<SessionCheckpoint>>,
}

impl TestStorage {
    fn add_checkpoint(&self, cp: SessionCheckpoint) {
        self.checkpoints.lock().unwrap().push(cp);
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

    async fn list_mined_undreamt_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        let cps = self.checkpoints.lock().unwrap();
        let result: Vec<String> = cps
            .iter()
            .filter(|cp| cp.mined && cp.dreaming_status != DreamingStatus::Completed)
            .map(|cp| cp.session_id.clone())
            .collect();
        Ok(result)
    }

    async fn mark_mined(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn update_dreaming_status(
        &self,
        session_id: &str,
        status: DreamingStatus,
    ) -> Result<(), PersistenceError> {
        let mut cps = self.checkpoints.lock().unwrap();
        if let Some(cp) = cps.iter_mut().find(|cp| cp.session_id == session_id) {
            cp.dreaming_status = status;
        }
        Ok(())
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

/// Dreaming pipeline does not reprocess sessions already marked Completed.
///
/// When `list_mined_undreamt_sessions()` returns empty (all sessions are
/// already Completed), `run_once()` should return Ok immediately without
/// attempting to process any entries.
#[tokio::test]
async fn test_dreaming_does_not_reprocess_completed() {
    let storage = TestStorage::default();

    // Session is mined=true but dreaming_status=Completed → should be skipped.
    let mut cp = SessionCheckpoint::new("sess-already-done".into());
    cp.mined = true;
    cp.dreaming_status = DreamingStatus::Completed;
    storage.add_checkpoint(cp);

    let pipeline = DreamingPipeline::new();
    let result = pipeline.run_once(&storage).await;
    assert!(result.is_ok(), "run_once should succeed: {result:?}");

    // Verify the session was NOT reprocessed (dreaming_status unchanged).
    let cps = storage.checkpoints.lock().unwrap();
    let cp = cps
        .iter()
        .find(|c| c.session_id == "sess-already-done")
        .unwrap();
    assert_eq!(
        cp.dreaming_status,
        DreamingStatus::Completed,
        "Completed session should not be reprocessed"
    );
}

/// Dreaming pipeline processes sessions that are mined but not yet dreamt.
#[tokio::test]
async fn test_dreaming_processes_mined_undreamt_sessions() {
    let storage = TestStorage::default();

    // mined=true, dreaming_status=Pending → should be processed.
    let mut cp = SessionCheckpoint::new("sess-pending".into());
    cp.mined = true;
    cp.dreaming_status = DreamingStatus::Pending;
    storage.add_checkpoint(cp);

    let pipeline = DreamingPipeline::new();
    let result = pipeline.run_once(&storage).await;
    assert!(result.is_ok(), "run_once should succeed: {result:?}");

    // The pipeline updates dreaming_status to Completed after processing.
    let cps = storage.checkpoints.lock().unwrap();
    let cp = cps.iter().find(|c| c.session_id == "sess-pending").unwrap();
    assert_eq!(
        cp.dreaming_status,
        DreamingStatus::Completed,
        "dreaming should mark session as Completed"
    );
}

/// Pipeline returns Ok immediately when there are no sessions to process.
#[tokio::test]
async fn test_dreaming_empty_storage_returns_ok() {
    let storage = TestStorage::default();
    let pipeline = DreamingPipeline::new();
    let result = pipeline.run_once(&storage).await;
    assert!(result.is_ok());
}
