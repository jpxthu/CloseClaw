//! Adapter: wraps the session [`PersistenceService`] in memory's narrow
//! [`MemoryStorage`] port.
//!
//! `closeclaw-memory` does not depend on `closeclaw-session`; the
//! composition layer (daemon) adapts the session persistence service to
//! the memory port at assembly time, converting between session-owned
//! and memory-owned types in both directions.

use std::sync::Arc;

use async_trait::async_trait;

use closeclaw_memory::storage::{
    CheckpointSnapshot, DreamingStatus as MemoryDreamingStatus, MemoryStorage, StorageError,
};
use closeclaw_session::persistence::{
    DreamingStatus as SessionDreamingStatus, PersistenceError, PersistenceService,
    SessionCheckpoint,
};

/// Map a session checkpoint onto memory's snapshot type.
///
/// Memory reads only `session_id` / `mined` / `dreaming_status`; all
/// other checkpoint fields are intentionally dropped.
pub(crate) fn checkpoint_snapshot(cp: &SessionCheckpoint) -> CheckpointSnapshot {
    CheckpointSnapshot::new(
        cp.session_id.clone(),
        cp.mined,
        session_status_to_memory(&cp.dreaming_status),
    )
}

/// Map a session dreaming status onto the memory-side enum.
pub(crate) fn session_status_to_memory(status: &SessionDreamingStatus) -> MemoryDreamingStatus {
    match status {
        SessionDreamingStatus::Pending => MemoryDreamingStatus::Pending,
        SessionDreamingStatus::InLight => MemoryDreamingStatus::InLight,
        SessionDreamingStatus::InRem => MemoryDreamingStatus::InRem,
        SessionDreamingStatus::InDeep => MemoryDreamingStatus::InDeep,
        SessionDreamingStatus::Completed => MemoryDreamingStatus::Completed,
    }
}

/// Map a memory-side dreaming status onto the session enum.
pub(crate) fn memory_status_to_session(status: MemoryDreamingStatus) -> SessionDreamingStatus {
    match status {
        MemoryDreamingStatus::Pending => SessionDreamingStatus::Pending,
        MemoryDreamingStatus::InLight => SessionDreamingStatus::InLight,
        MemoryDreamingStatus::InRem => SessionDreamingStatus::InRem,
        MemoryDreamingStatus::InDeep => SessionDreamingStatus::InDeep,
        MemoryDreamingStatus::Completed => SessionDreamingStatus::Completed,
    }
}

/// Map a session persistence error onto the memory port error.
fn storage_error(err: PersistenceError) -> StorageError {
    match err {
        PersistenceError::NotFound(session_id) => StorageError::NotFound(session_id),
        other => StorageError::Backend(other.to_string()),
    }
}

/// Wraps a session [`PersistenceService`] in memory's [`MemoryStorage`]
/// port for the daemon composition root.
pub(crate) struct MemoryStorageAdapter {
    inner: Arc<dyn PersistenceService>,
}

impl MemoryStorageAdapter {
    /// Wrap the given session persistence service.
    pub(crate) fn new(inner: Arc<dyn PersistenceService>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl MemoryStorage for MemoryStorageAdapter {
    async fn load_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<CheckpointSnapshot>, StorageError> {
        self.inner
            .load_checkpoint(session_id)
            .await
            .map(|opt| opt.map(|cp| checkpoint_snapshot(&cp)))
            .map_err(storage_error)
    }

    async fn mark_mined(&self, session_id: &str) -> Result<(), StorageError> {
        self.inner
            .mark_mined(session_id)
            .await
            .map_err(storage_error)
    }

    async fn list_mined_undreamt_sessions(&self) -> Result<Vec<String>, StorageError> {
        self.inner
            .list_mined_undreamt_sessions()
            .await
            .map_err(storage_error)
    }

    async fn update_dreaming_status(
        &self,
        session_id: &str,
        status: MemoryDreamingStatus,
    ) -> Result<(), StorageError> {
        self.inner
            .update_dreaming_status(session_id, memory_status_to_session(status))
            .await
            .map_err(storage_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dreaming_status_round_trip_is_lossless() {
        let all = [
            SessionDreamingStatus::Pending,
            SessionDreamingStatus::InLight,
            SessionDreamingStatus::InRem,
            SessionDreamingStatus::InDeep,
            SessionDreamingStatus::Completed,
        ];
        for session_status in all {
            let memory_status = session_status_to_memory(&session_status);
            assert_eq!(
                memory_status_to_session(memory_status),
                session_status,
                "round trip must preserve {session_status}"
            );
        }
    }

    #[test]
    fn test_checkpoint_snapshot_maps_consumed_fields_only() {
        let mut cp = SessionCheckpoint::new("sess-1".into());
        cp.mined = true;
        cp.dreaming_status = SessionDreamingStatus::InDeep;
        let snapshot = checkpoint_snapshot(&cp);
        assert_eq!(snapshot.session_id, "sess-1");
        assert!(snapshot.mined);
        assert_eq!(
            snapshot.dreaming_status,
            MemoryDreamingStatus::InDeep,
            "dreaming status must map semantically"
        );
    }
}
