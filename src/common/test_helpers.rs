//! Shared test helpers for daemon and integration tests.

use std::io;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::session::persistence::{
    DreamingStatus, PersistenceError, PersistenceService, SessionCheckpoint,
};

/// Write the 5 mandatory config files (models.json, channels.json,
/// gateway.json, plugins.json, system.json) into `dir`.
///
/// Reused across daemon unit tests, E2E tests, and integration tests
/// to avoid duplicating the same for-loop in every test helper.
pub fn write_mandatory_configs(dir: &std::path::Path) -> io::Result<()> {
    for name in &[
        "models.json",
        "channels.json",
        "gateway.json",
        "plugins.json",
        "system.json",
    ] {
        std::fs::write(
            dir.join(name),
            serde_json::json!({"version": "1.0"}).to_string(),
        )?;
    }
    Ok(())
}

// ── Shared TestStorage ───────────────────────────────────────────────────

/// Minimal in-memory [`PersistenceService`] for unit tests.
///
/// Provides a basic implementation backed by `Vec` storage, covering the
/// CRUD and dreaming/mining methods needed by memory, daemon, and other
/// module tests. Methods not exercised by tests return sensible defaults.
///
/// # Usage
///
/// ```ignore
/// use crate::common::test_helpers::TestStorage;
///
/// let storage = TestStorage::default();
/// storage.add_checkpoint(SessionCheckpoint::new("s1".into()));
/// ```
#[derive(Debug, Default)]
pub struct TestStorage {
    /// Active / general checkpoints.
    pub checkpoints: Mutex<Vec<SessionCheckpoint>>,
    /// Archived checkpoints (used by `list_archived_unmined_sessions`).
    pub archived: Mutex<Vec<SessionCheckpoint>>,
    /// Tracks which sessions were marked mined (for assertion in tests).
    pub mined_ids: Mutex<Vec<String>>,
}

impl TestStorage {
    /// Insert a checkpoint into the active store.
    pub fn add_checkpoint(&self, cp: SessionCheckpoint) {
        self.checkpoints.lock().unwrap().push(cp);
    }

    /// Insert a checkpoint into the archived store.
    pub fn add_archived(&self, cp: SessionCheckpoint) {
        self.archived.lock().unwrap().push(cp);
    }

    /// Return a clone of the mined session IDs recorded so far.
    pub fn mined_ids(&self) -> Vec<String> {
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

    async fn load_archived_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self
            .archived
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

    async fn list_archived_unmined_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(self
            .archived
            .lock()
            .unwrap()
            .iter()
            .filter(|cp| !cp.mined)
            .map(|cp| cp.session_id.clone())
            .collect())
    }

    async fn list_mined_undreamt_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        let cps = self.checkpoints.lock().unwrap();
        Ok(cps
            .iter()
            .filter(|cp| cp.mined && cp.dreaming_status != DreamingStatus::Completed)
            .map(|cp| cp.session_id.clone())
            .collect())
    }

    async fn mark_mined(&self, session_id: &str) -> Result<(), PersistenceError> {
        self.mined_ids.lock().unwrap().push(session_id.into());
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
