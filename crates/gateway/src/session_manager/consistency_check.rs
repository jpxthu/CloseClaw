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
        let storage = {
            let guard = self.storage.read().await;
            match guard.as_ref() {
                Some(s) => Arc::clone(s),
                None => return Ok(ConsistencyCheckResult::default()),
            }
        };
        let result = storage.run_consistency_check().await?;
        info!(
            deleted_orphaned_records = result.deleted_orphaned_records,
            deleted_orphaned_files = result.deleted_orphaned_files,
            "consistency check completed"
        );
        Ok(result)
    }

    /// Spawn a background task that runs periodic consistency checks.
    ///
    /// The first check is skipped (the startup full scan is done separately
    /// by the caller). Subsequent checks run at the given `interval`.
    /// The task runs at a low priority and does not block request processing.
    pub fn spawn_periodic_consistency_check(self: &Arc<Self>, interval: Duration) {
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // Skip the first immediate tick — startup scan is done by caller.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                if let Err(e) = mgr.run_consistency_check().await {
                    warn!(error = %e, "periodic consistency check failed");
                }
            }
        });
    }
}
