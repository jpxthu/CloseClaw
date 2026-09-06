//! Timeout behavior tests for session recovery.
//!
//! Verifies that `tokio::time::timeout` wrapping `SessionRecoveryService::recover()`
//! behaves correctly: triggers on slow storage, completes on fast storage, and
//! propagates errors without timeout when storage fails quickly.

#[cfg(test)]
mod tests {
    use crate::persistence::{
        DreamingStatus, PendingOperation, PendingOperationDetail, PendingOperationStatus,
        PendingOperationType, PersistenceError, PersistenceService, SessionCheckpoint,
    };
    use crate::recovery::SessionRecoveryService;
    use crate::storage::memory::MemoryStorage;
    use chrono::Utc;
    use std::sync::Arc;

    // ── Mock storages ──────────────────────────────────────────────────────

    /// Mock storage that delays on `list_active_sessions()` to simulate slow backend.
    struct SlowStorage {
        inner: MemoryStorage,
        /// Delay injected before returning from `list_active_sessions()`.
        delay: std::time::Duration,
    }

    impl SlowStorage {
        fn new(delay: std::time::Duration) -> Self {
            Self {
                inner: MemoryStorage::new(),
                delay,
            }
        }
    }

    #[async_trait::async_trait]
    impl PersistenceService for SlowStorage {
        async fn save_checkpoint(
            &self,
            checkpoint: &SessionCheckpoint,
        ) -> Result<(), PersistenceError> {
            self.inner.save_checkpoint(checkpoint).await
        }

        async fn load_checkpoint(
            &self,
            session_id: &str,
        ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
            self.inner.load_checkpoint(session_id).await
        }

        async fn delete_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError> {
            self.inner.delete_checkpoint(session_id).await
        }

        async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            tokio::time::sleep(self.delay).await;
            self.inner.list_active_sessions().await
        }

        async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            self.inner.list_archived_sessions().await
        }

        async fn list_migrating_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            self.inner.list_migrating_sessions().await
        }

        async fn archive_checkpoint(
            &self,
            checkpoint: &SessionCheckpoint,
        ) -> Result<(), PersistenceError> {
            self.inner.archive_checkpoint(checkpoint).await
        }

        async fn restore_checkpoint(
            &self,
            session_id: &str,
        ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
            self.inner.restore_checkpoint(session_id).await
        }

        async fn load_archived_checkpoint(
            &self,
            session_id: &str,
        ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
            self.inner.load_archived_checkpoint(session_id).await
        }

        async fn list_children_sessions(
            &self,
            parent_session_id: &str,
        ) -> Result<Vec<String>, PersistenceError> {
            self.inner.list_children_sessions(parent_session_id).await
        }

        async fn list_archived_unmined_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            self.inner.list_archived_unmined_sessions().await
        }

        async fn list_mined_undreamt_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            self.inner.list_mined_undreamt_sessions().await
        }

        async fn mark_mined(&self, session_id: &str) -> Result<(), PersistenceError> {
            self.inner.mark_mined(session_id).await
        }

        async fn update_dreaming_status(
            &self,
            session_id: &str,
            status: DreamingStatus,
        ) -> Result<(), PersistenceError> {
            self.inner.update_dreaming_status(session_id, status).await
        }

        async fn save_snapshot_metas(
            &self,
            session_id: &str,
            metas: &[crate::run_health::SnapshotMeta],
        ) -> Result<(), PersistenceError> {
            self.inner.save_snapshot_metas(session_id, metas).await
        }

        async fn load_snapshot_metas(
            &self,
            session_id: &str,
        ) -> Result<Vec<crate::run_health::SnapshotMeta>, PersistenceError> {
            self.inner.load_snapshot_metas(session_id).await
        }
    }

    /// Mock storage that always returns an error on `list_active_sessions()`.
    struct FailStorage;

    #[async_trait::async_trait]
    impl PersistenceService for FailStorage {
        async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
            Ok(())
        }
        async fn load_checkpoint(
            &self,
            _: &str,
        ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
            Ok(None)
        }
        async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
            Ok(())
        }
        async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            Err(PersistenceError::Lock("storage unavailable".to_string()))
        }
        async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            Err(PersistenceError::Lock("storage unavailable".to_string()))
        }
        async fn list_migrating_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }
        async fn archive_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
            Ok(())
        }
        async fn restore_checkpoint(
            &self,
            _: &str,
        ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
            Ok(None)
        }
        async fn load_archived_checkpoint(
            &self,
            _: &str,
        ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
            Ok(None)
        }
        async fn list_children_sessions(&self, _: &str) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }
        async fn list_archived_unmined_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }
        async fn list_mined_undreamt_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }
        async fn mark_mined(&self, _: &str) -> Result<(), PersistenceError> {
            Ok(())
        }
        async fn update_dreaming_status(
            &self,
            _: &str,
            _: DreamingStatus,
        ) -> Result<(), PersistenceError> {
            Ok(())
        }
        async fn save_snapshot_metas(
            &self,
            _: &str,
            _: &[crate::run_health::SnapshotMeta],
        ) -> Result<(), PersistenceError> {
            Ok(())
        }
        async fn load_snapshot_metas(
            &self,
            _: &str,
        ) -> Result<Vec<crate::run_health::SnapshotMeta>, PersistenceError> {
            Ok(Vec::new())
        }
    }

    // ── Timeout behavior tests ─────────────────────────────────────────────

    /// Verify that `tokio::time::timeout` wrapping `recover()` triggers
    /// when storage is slow, returning empty results without panic.
    #[tokio::test]
    async fn test_timeout_recover_exceeds_deadline() {
        let storage = Arc::new(SlowStorage::new(std::time::Duration::from_secs(20)));
        let service = SessionRecoveryService::new(storage);

        let result =
            tokio::time::timeout(std::time::Duration::from_millis(50), service.recover()).await;

        assert!(result.is_err(), "timeout should have fired");
        // Elapsed means the timeout triggered — equivalent to the daemon's
        // timeout handler returning (Vec::new(), Vec::new()).
    }

    /// Verify that `tokio::time::timeout` wrapping `recover()` completes
    /// normally when storage responds quickly.
    #[tokio::test]
    async fn test_timeout_recover_within_deadline() {
        let storage = Arc::new(SlowStorage::new(std::time::Duration::from_millis(5)));
        let service = SessionRecoveryService::new(storage);

        let result =
            tokio::time::timeout(std::time::Duration::from_secs(10), service.recover()).await;

        assert!(result.is_ok(), "should complete within deadline");
        let report = result.unwrap().unwrap();
        assert!(report.recovered.is_empty());
        assert!(report.failed.is_empty());
    }

    /// Verify that when storage returns an error, the timeout wrapper
    /// still yields the error (not a timeout).
    #[tokio::test]
    async fn test_timeout_recover_error_within_deadline() {
        let storage = Arc::new(FailStorage);
        let service = SessionRecoveryService::new(storage);

        let result =
            tokio::time::timeout(std::time::Duration::from_secs(10), service.recover()).await;

        assert!(result.is_ok(), "should complete within deadline");
        let inner = result.unwrap();
        assert!(inner.is_err(), "storage error should propagate");
    }

    /// Verify the daemon-side timeout pattern: match on timeout vs result.
    /// This mirrors the exact match expression in `daemon/mod.rs`.
    #[tokio::test]
    async fn test_daemon_timeout_pattern_slow_storage() {
        let storage = Arc::new(SlowStorage::new(std::time::Duration::from_secs(20)));
        let service = SessionRecoveryService::new(storage);

        let recovery_result =
            tokio::time::timeout(std::time::Duration::from_millis(50), service.recover()).await;

        let (dirty, migrated) = match recovery_result {
            Ok(Ok(report)) => (report.dirty_sessions, report.migrated_sessions),
            Ok(Err(_e)) => (Vec::new(), Vec::new()),
            Err(_timeout) => (Vec::new(), Vec::new()),
        };

        assert!(dirty.is_empty());
        assert!(migrated.is_empty());
    }

    /// Verify the daemon-side timeout pattern: fast storage produces results.
    #[tokio::test]
    async fn test_daemon_timeout_pattern_fast_storage() {
        let storage = Arc::new(SlowStorage::new(std::time::Duration::from_millis(5)));

        // Insert a dirty session so we get non-empty results
        let now = Utc::now();
        let dirty = SessionCheckpoint::new("dirty-fast".into()).with_pending_operations(vec![
            PendingOperation {
                status: PendingOperationStatus::Running,
                op_id: "op1".into(),
                op_type: PendingOperationType::ToolCall,
                detail: PendingOperationDetail::ToolCall {
                    tool_name: "exec".into(),
                    args_summary: String::new(),
                },
                created_at: now,
            },
        ]);
        storage.inner.save_checkpoint(&dirty).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));

        let recovery_result =
            tokio::time::timeout(std::time::Duration::from_secs(10), service.recover()).await;

        let (dirty_sessions, migrated_sessions) = match recovery_result {
            Ok(Ok(report)) => (report.dirty_sessions, report.migrated_sessions),
            Ok(Err(_e)) => (Vec::new(), Vec::new()),
            Err(_timeout) => (Vec::new(), Vec::new()),
        };

        assert_eq!(dirty_sessions, vec!["dirty-fast".to_string()]);
        assert!(migrated_sessions.is_empty());
    }
}
