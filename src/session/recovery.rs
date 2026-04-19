//! Session recovery service
//!
//! Provides functionality to recover sessions from persisted checkpoints
//! during gateway startup.

use crate::session::persistence::{PersistenceError, PersistenceService, SessionCheckpoint};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Session recovery service — recovers sessions from persisted checkpoints
pub struct SessionRecoveryService<S: PersistenceService> {
    storage: Arc<S>,
    /// Callback to restore a session from checkpoint
    /// The closure receives the session_id and checkpoint, and should restore the session state.
    restore_fn: RwLock<
        Option<Box<dyn Fn(&str, &SessionCheckpoint) -> Result<(), PersistenceError> + Send + Sync>>,
    >,
}

impl<S: PersistenceService> SessionRecoveryService<S> {
    /// Create a new SessionRecoveryService
    pub fn new(storage: Arc<S>) -> Self {
        Self {
            storage,
            restore_fn: RwLock::new(None),
        }
    }

    /// Set the restore callback
    ///
    /// The callback will be invoked for each session during recovery.
    pub async fn set_restore_callback<F>(&self, callback: F)
    where
        F: Fn(&str, &SessionCheckpoint) -> Result<(), PersistenceError> + Send + Sync + 'static,
    {
        let mut restore_fn = self.restore_fn.write().await;
        *restore_fn = Some(Box::new(callback));
    }

    /// Execute the recovery process
    ///
    /// Scans all active sessions from storage and attempts to recover each one.
    pub async fn recover(&self) -> Result<RecoveryReport, PersistenceError> {
        let active_sessions = self.storage.list_active_sessions().await?;
        let mut recovered = Vec::new();
        let mut failed = Vec::new();

        for session_id in active_sessions {
            match self.recover_session(&session_id).await {
                Ok(()) => recovered.push(session_id.clone()),
                Err(e) => {
                    tracing::error!(
                        session_id = %session_id,
                        "Failed to recover session: {}",
                        e
                    );
                    failed.push(session_id);
                }
            }
        }

        Ok(RecoveryReport { recovered, failed })
    }

    /// Recover a single session
    async fn recover_session(&self, session_id: &str) -> Result<(), PersistenceError> {
        let checkpoint = self
            .storage
            .load_checkpoint(session_id)
            .await?
            .ok_or_else(|| PersistenceError::NotFound(session_id.to_string()))?;

        let restore_fn = self.restore_fn.read().await;
        if let Some(callback) = restore_fn.as_ref() {
            callback(session_id, &checkpoint)?;
        }

        Ok(())
    }

    /// Get the storage reference
    pub fn storage(&self) -> &S {
        &*self.storage
    }
}

/// Recovery report containing results of the recovery process
#[derive(Debug)]
pub struct RecoveryReport {
    /// List of session IDs that were successfully recovered
    pub recovered: Vec<String>,
    /// List of session IDs that failed to recover
    pub failed: Vec<String>,
}

impl RecoveryReport {
    /// Returns true if all sessions were recovered successfully
    pub fn is_full_success(&self) -> bool {
        self.failed.is_empty()
    }

    /// Returns the total number of sessions processed
    pub fn total(&self) -> usize {
        self.recovered.len() + self.failed.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::persistence::{ReasoningMode, ReasoningModeState};
    use crate::session::storage::memory::MemoryStorage;
    use chrono::Utc;

    fn create_test_checkpoint(session_id: &str) -> SessionCheckpoint {
        SessionCheckpoint {
            session_id: session_id.to_string(),
            last_message_id: Some("msg123".to_string()),
            mode_state: ReasoningModeState {
                current_step: 1,
                total_steps: 3,
                step_messages: vec!["Step 1".to_string()],
                is_complete: false,
            },
            pending_messages: Vec::new(),
            mode: ReasoningMode::Plan,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            ttl_seconds: 604800,
        }
    }

    #[tokio::test]
    async fn test_recovery_report_is_full_success() {
        let report = RecoveryReport {
            recovered: vec!["s1".to_string(), "s2".to_string()],
            failed: Vec::new(),
        };
        assert!(report.is_full_success());
        assert_eq!(report.total(), 2);
    }

    #[tokio::test]
    async fn test_recovery_report_has_failures() {
        let report = RecoveryReport {
            recovered: vec!["s1".to_string()],
            failed: vec!["s2".to_string()],
        };
        assert!(!report.is_full_success());
        assert_eq!(report.total(), 2);
    }

    #[tokio::test]
    async fn test_recovery_service_recover_empty() {
        let storage = Arc::new(MemoryStorage::new());
        let service = SessionRecoveryService::new(storage);

        let report = service.recover().await.unwrap();
        assert!(report.recovered.is_empty());
        assert!(report.failed.is_empty());
    }

    #[tokio::test]
    async fn test_recovery_service_recover_with_callback() {
        let storage = Arc::new(MemoryStorage::new());

        // Pre-populate storage with checkpoints
        storage
            .save_checkpoint(&create_test_checkpoint("session1"))
            .await
            .unwrap();
        storage
            .save_checkpoint(&create_test_checkpoint("session2"))
            .await
            .unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));

        // Track which sessions were restored
        let restored = Arc::new(std::sync::Mutex::new(Vec::new()));
        let restored_clone = Arc::clone(&restored);

        service
            .set_restore_callback(move |session_id, _checkpoint| {
                restored_clone.lock().unwrap().push(session_id.to_string());
                Ok(())
            })
            .await;

        let report = service.recover().await.unwrap();

        assert_eq!(report.recovered.len(), 2);
        assert!(report.failed.is_empty());

        let mut restored_sessions = restored.lock().unwrap();
        restored_sessions.sort();
        assert_eq!(restored_sessions[0], "session1");
        assert_eq!(restored_sessions[1], "session2");
    }

    #[tokio::test]
    async fn test_recovery_service_recover_not_found() {
        let storage = Arc::new(MemoryStorage::new());
        let service = SessionRecoveryService::new(Arc::clone(&storage));

        // Don't set any restore callback, but storage has a checkpoint
        storage
            .save_checkpoint(&create_test_checkpoint("orphan"))
            .await
            .unwrap();

        // Recover should still succeed even without callback
        let report = service.recover().await.unwrap();
        assert_eq!(report.recovered.len(), 1);
    }
}
