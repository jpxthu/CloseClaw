//! Shared test helpers for memory crate unit tests.

use std::sync::Mutex;

use rusqlite::params;

use crate::miner::MiningEventCategory;

use async_trait::async_trait;

use closeclaw_session::persistence::{
    DreamingStatus, PersistenceError, PersistenceService, SessionCheckpoint,
};

/// Minimal in-memory [`PersistenceService`] for unit tests.
#[derive(Debug, Default)]
pub struct TestStorage {
    /// Active / general checkpoints.
    pub checkpoints: Mutex<Vec<SessionCheckpoint>>,
    /// Archived checkpoints.
    pub archived: Mutex<Vec<SessionCheckpoint>>,
    /// Tracks which sessions were marked mined.
    pub mined_ids: Mutex<Vec<String>>,
}

impl TestStorage {
    /// Insert a checkpoint into the active store.
    pub fn add_checkpoint(&self, cp: SessionCheckpoint) {
        self.checkpoints.lock().unwrap().push(cp);
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
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
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

// ── SQLite test helpers (forgetting / miner_tests) ─────────────────

/// Insert an event directly with a specific expires_at, bypassing
/// `write_to_sqlite` (which computes expires_at from `Utc::now()`).
pub(crate) fn insert_event(
    conn: &rusqlite::Connection,
    session_id: &str,
    agent_id: &str,
    category: MiningEventCategory,
    expires_at: i64,
) -> i64 {
    let ts = 500i64; // fixed timestamp for determinism
    conn.query_row(
        "INSERT INTO events (title, summary, content, category, lesson, \
         source_session_id, agent_id, timestamp, updated_at, expires_at) \
         VALUES ('t', 's', 'b', ?1, NULL, ?2, ?3, ?4, ?4, ?5) \
         RETURNING id",
        params![category.to_string(), session_id, agent_id, ts, expires_at],
        |row| row.get(0),
    )
    .unwrap()
}

/// Insert an entity and link it to an event via event_entities.
pub(crate) fn insert_entity_with_event(
    conn: &rusqlite::Connection,
    agent_id: &str,
    event_id: i64,
    name: &str,
) {
    let norm_name = name.to_lowercase().replace(' ', "_");
    conn.execute(
        "INSERT OR IGNORE INTO entities (agent_id, type, name, normalized_name, description) \
         VALUES (?1, 'subject', ?2, ?3, 'desc')",
        params![agent_id, name, norm_name],
    )
    .unwrap();
    let entity_id: i64 = conn
        .query_row(
            "SELECT id FROM entities WHERE agent_id = ?1 AND type = 'subject' \
             AND normalized_name = ?2",
            params![agent_id, norm_name],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO event_entities (event_id, entity_id) VALUES (?1, ?2)",
        params![event_id, entity_id],
    )
    .unwrap();
}
