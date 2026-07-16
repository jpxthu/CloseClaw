//! Startup and periodic data consistency checking.
//!
//! `run_consistency_check()` performs a bidirectional scan between SQLite
//! and the file system to detect and clean up orphaned records/files.

use super::SessionManager;
use closeclaw_session::persistence::{ConsistencyCheckResult, PersistenceError};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

impl SessionManager {
    /// Run a one-shot consistency check between SQLite and the file system.
    ///
    /// - SQLite → File system: records with missing transcript files are deleted.
    /// - File system → SQLite: orphan transcript files are deleted.
    pub async fn run_consistency_check(&self) -> Result<ConsistencyCheckResult, PersistenceError> {
        let cm = {
            let guard = self.checkpoint_manager.read().await;
            match guard.as_ref() {
                Some(cm) => std::sync::Arc::clone(cm),
                None => return Ok(ConsistencyCheckResult::default()),
            }
        };
        let result = cm.storage().run_consistency_check().await?;
        info!(
            deleted_orphaned_records = result.deleted_orphaned_records,
            deleted_orphaned_files = result.deleted_orphaned_files,
            "consistency check completed"
        );
        Ok(result)
    }

    /// Run an incremental consistency check since the last scan.
    ///
    /// Only examines SQLite records with `last_message_at > since` and
    /// transcript files with `mtime > since`, reducing I/O overhead
    /// compared to the full bidirectional scan.
    async fn run_incremental_since_last_check(
        &self,
    ) -> Result<ConsistencyCheckResult, PersistenceError> {
        let since = {
            let guard = self.last_consistency_check_time.lock().unwrap();
            guard.unwrap_or(0)
        };
        let cm = {
            let guard = self.checkpoint_manager.read().await;
            match guard.as_ref() {
                Some(cm) => std::sync::Arc::clone(cm),
                None => return Ok(ConsistencyCheckResult::default()),
            }
        };
        let result = cm
            .storage()
            .run_incremental_consistency_check(since)
            .await?;
        info!(
            since = since,
            deleted_orphaned_records = result.deleted_orphaned_records,
            deleted_orphaned_files = result.deleted_orphaned_files,
            "incremental consistency check completed"
        );
        // Update timestamp for the next incremental scan.
        *self.last_consistency_check_time.lock().unwrap() = Some(chrono::Utc::now().timestamp());
        Ok(result)
    }

    /// Spawn a background task that runs periodic consistency checks.
    ///
    /// Uses incremental scanning: each run only examines records and files
    /// that changed since the previous scan. The first check is skipped
    /// (the startup full scan is done separately by the caller). Subsequent
    /// checks run at the given `interval`.
    pub fn spawn_periodic_consistency_check(self: &Arc<Self>, interval: Duration) {
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // Skip the first immediate tick — startup scan is done by caller.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                if let Err(e) = mgr.run_incremental_since_last_check().await {
                    warn!(error = %e, "periodic incremental consistency check failed");
                }
            }
        });
    }
}
