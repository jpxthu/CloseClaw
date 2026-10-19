//! Shared mock persistence for gateway tests.
//!
//! [`SnapshotMockPersist`] stores checkpoints in an in-memory `HashMap` keyed
//! by session id (load returns the last saved checkpoint for that session)
//! and records a [`SaveSnapshot`] of outbound counters on every save. New
//! gateway persist mocks should reuse this implementation when the case
//! semantics are a subset of its capabilities; extend it instead of adding
//! local copies.

use closeclaw_session::persistence::{
    PendingOperationType, PersistenceError, PersistenceService, SessionCheckpoint,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub(crate) struct SaveSnapshot {
    pub(crate) outbound_ops_count: usize,
    #[allow(dead_code)] // recorded for diagnostics; no test reads it yet
    pub(crate) outbound_pending_count: usize,
}

pub(crate) struct SnapshotMockPersist {
    pub(crate) checkpoints: Mutex<HashMap<String, SessionCheckpoint>>,
    pub(crate) snapshots: Arc<StdMutex<Vec<SaveSnapshot>>>,
    /// Fail on the Nth save call (0-indexed).
    fail_on_save: StdMutex<Option<usize>>,
    call_count: StdMutex<usize>,
}

impl SnapshotMockPersist {
    pub(crate) fn new() -> Self {
        Self::with_fail_on_save_impl(None)
    }

    pub(crate) fn with_fail_on_save(n: usize) -> Self {
        Self::with_fail_on_save_impl(Some(n))
    }

    fn with_fail_on_save_impl(fail_on_save: Option<usize>) -> Self {
        Self {
            checkpoints: Mutex::new(HashMap::new()),
            snapshots: Arc::new(StdMutex::new(Vec::new())),
            fail_on_save: StdMutex::new(fail_on_save),
            call_count: StdMutex::new(0),
        }
    }

    pub(crate) fn snapshots(&self) -> Vec<SaveSnapshot> {
        self.snapshots.lock().unwrap().clone()
    }
}

/// Create in-memory mock persistence whose checkpoint for `session_id`
/// binds `sender` as the session Owner (checkpoint `sender_id`) — the
/// checkpoint Owner criterion (`SessionManager::get_sender_id`) used by
/// the gateway Owner gates in tests.
///
/// `sender = None` writes no checkpoint: the session has no bound Owner
/// and nobody qualifies for the Owner shortcut.
pub(crate) async fn persist_with_checkpoint_sender(
    session_id: &str,
    sender: Option<&str>,
) -> Arc<dyn PersistenceService> {
    let persist = Arc::new(SnapshotMockPersist::new());
    if let Some(sender) = sender {
        let mut cp = SessionCheckpoint::new(session_id.to_owned());
        cp.sender_id = Some(sender.to_owned());
        persist.save_checkpoint(&cp).await.unwrap();
    }
    persist as Arc<dyn PersistenceService>
}

#[async_trait::async_trait]
impl PersistenceService for SnapshotMockPersist {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        let n = {
            let mut count = self.call_count.lock().unwrap();
            let n = *count;
            *count += 1;
            n
        };
        if let Some(fail_at) = *self.fail_on_save.lock().unwrap() {
            if n == fail_at {
                return Err(PersistenceError::Lock(format!(
                    "simulated failure on save #{}",
                    n
                )));
            }
        }
        let outbound_ops = cp
            .pending_operations
            .iter()
            .filter(|op| op.op_type == PendingOperationType::OutboundMessage)
            .count();
        self.snapshots.lock().unwrap().push(SaveSnapshot {
            outbound_ops_count: outbound_ops,
            outbound_pending_count: cp.outbound_pending.len(),
        });
        self.checkpoints
            .lock()
            .await
            .insert(cp.session_id.clone(), cp.clone());
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.checkpoints.lock().await.get(sid).cloned())
    }

    async fn delete_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn purge_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn invalidate_session(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn archive_checkpoint(&self, _cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn restore_checkpoint(
        &self,
        _sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn list_idle_sessions_for_agent(
        &self,
        _: &str,
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_expired_archived_sessions_for_agent(
        &self,
        _: &str,
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
}
