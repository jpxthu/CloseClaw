//! Tests for data consistency checking.

use super::SessionManager;
use closeclaw_session::persistence::{
    ConsistencyCheckResult, PersistenceError, PersistenceService, SessionCheckpoint,
};
use std::sync::Arc;

/// Mock that tracks deleted records and files.
struct ConsistencyMock {
    active_sessions: Vec<String>,
    archived_sessions: Vec<String>,
    /// Checkpoint data keyed by session_id.
    checkpoints: std::collections::HashMap<String, SessionCheckpoint>,
    /// Records deleted via `delete_checkpoint`.
    deleted: std::sync::Mutex<Vec<String>>,
}

impl ConsistencyMock {
    fn new(active: Vec<String>, archived: Vec<String>) -> Self {
        Self {
            active_sessions: active,
            archived_sessions: archived,
            checkpoints: std::collections::HashMap::new(),
            deleted: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn with_checkpoint(mut self, cp: SessionCheckpoint) -> Self {
        self.checkpoints.insert(cp.session_id.clone(), cp);
        self
    }
}

#[async_trait::async_trait]
impl PersistenceService for ConsistencyMock {
    async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.checkpoints.get(id).cloned())
    }
    async fn delete_checkpoint(&self, id: &str) -> Result<(), PersistenceError> {
        self.deleted.lock().unwrap().push(id.to_string());
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(self.active_sessions.clone())
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(self.archived_sessions.clone())
    }
    async fn restore_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn run_consistency_check(&self) -> Result<ConsistencyCheckResult, PersistenceError> {
        // Simulate: all active sessions are orphaned (no transcript files).
        let mut result = ConsistencyCheckResult::default();
        for id in &self.active_sessions {
            self.deleted.lock().unwrap().push(id.clone());
            result.deleted_orphaned_records += 1;
        }
        Ok(result)
    }
}

fn test_config() -> crate::GatewayConfig {
    crate::GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        ..Default::default()
    }
}

#[tokio::test]
async fn test_run_consistency_check_returns_result() {
    let mock = Arc::new(
        ConsistencyMock::new(vec!["sid-1".into()], vec![]).with_checkpoint(
            SessionCheckpoint::new("sid-1".into())
                .with_platform("feishu".into())
                .with_peer_id("agent-a".into())
                .with_agent_id("agent-a".into()),
        ),
    );
    let mgr = SessionManager::new(&test_config(), Some(mock), None, Default::default());

    let result = mgr.run_consistency_check().await.unwrap();
    // Mock deletes all active sessions as orphaned.
    assert_eq!(result.deleted_orphaned_records, 1);
}

#[tokio::test]
async fn test_run_consistency_check_no_storage() {
    let mgr = SessionManager::new(&test_config(), None, None, Default::default());
    let result = mgr.run_consistency_check().await.unwrap();
    assert_eq!(result.deleted_orphaned_records, 0);
    assert_eq!(result.deleted_orphaned_files, 0);
}

/// Verify that `ConsistencyCheckResult` defaults to zero.
#[test]
fn test_consistency_check_result_default() {
    let r = ConsistencyCheckResult::default();
    assert_eq!(r.deleted_orphaned_records, 0);
    assert_eq!(r.deleted_orphaned_files, 0);
}

/// Verify that `ConsistencyCheckResult` is cloneable.
#[test]
fn test_consistency_check_result_clone() {
    let mut r = ConsistencyCheckResult::default();
    r.deleted_orphaned_records = 3;
    r.deleted_orphaned_files = 2;
    let r2 = r.clone();
    assert_eq!(r2.deleted_orphaned_records, 3);
    assert_eq!(r2.deleted_orphaned_files, 2);
}

/// Mock that returns success with no orphans (clean state).
struct CleanConsistencyMock;

#[async_trait::async_trait]
impl PersistenceService for CleanConsistencyMock {
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
        Ok(vec![])
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn run_consistency_check(&self) -> Result<ConsistencyCheckResult, PersistenceError> {
        Ok(ConsistencyCheckResult {
            deleted_orphaned_records: 0,
            deleted_orphaned_files: 0,
        })
    }
}

#[tokio::test]
async fn test_run_consistency_check_clean_state() {
    let mock = Arc::new(CleanConsistencyMock);
    let mgr = SessionManager::new(&test_config(), Some(mock), None, Default::default());
    let result = mgr.run_consistency_check().await.unwrap();
    assert_eq!(result.deleted_orphaned_records, 0);
    assert_eq!(result.deleted_orphaned_files, 0);
}

// ── Step 1.5: consistency check behavior verification ─────────────────────

/// Mock that simulates both orphan types:
/// - SQLite record without file → deleted_orphaned_records
/// - File without SQLite record → deleted_orphaned_files
struct BidirectionalMock {
    orphaned_records: u64,
    orphaned_files: u64,
    deleted_records: std::sync::Mutex<Vec<String>>,
    deleted_files: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl PersistenceService for BidirectionalMock {
    async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn delete_checkpoint(&self, id: &str) -> Result<(), PersistenceError> {
        self.deleted_records.lock().unwrap().push(id.to_string());
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn restore_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn run_consistency_check(&self) -> Result<ConsistencyCheckResult, PersistenceError> {
        // Simulate: orphaned records cleaned and orphaned files deleted.
        for i in 0..self.orphaned_records {
            self.deleted_records
                .lock()
                .unwrap()
                .push(format!("orphan-record-{}", i));
        }
        for i in 0..self.orphaned_files {
            self.deleted_files
                .lock()
                .unwrap()
                .push(format!("orphan-file-{}.jsonl", i));
        }
        Ok(ConsistencyCheckResult {
            deleted_orphaned_records: self.orphaned_records,
            deleted_orphaned_files: self.orphaned_files,
        })
    }
}

/// Verify that orphaned SQLite records (file missing) are detected and cleaned.
#[tokio::test]
async fn test_consistency_check_cleans_orphaned_records() {
    let mock = Arc::new(BidirectionalMock {
        orphaned_records: 3,
        orphaned_files: 0,
        deleted_records: std::sync::Mutex::new(Vec::new()),
        deleted_files: std::sync::Mutex::new(Vec::new()),
    });
    let mgr = SessionManager::new(&test_config(), Some(mock.clone()), None, Default::default());
    let result = mgr.run_consistency_check().await.unwrap();
    assert_eq!(result.deleted_orphaned_records, 3);
    assert_eq!(result.deleted_orphaned_files, 0);
    let deleted = mock.deleted_records.lock().unwrap();
    assert_eq!(deleted.len(), 3);
}

/// Verify that orphaned files (no SQLite record) are detected and cleaned.
#[tokio::test]
async fn test_consistency_check_cleans_orphaned_files() {
    let mock = Arc::new(BidirectionalMock {
        orphaned_records: 0,
        orphaned_files: 2,
        deleted_records: std::sync::Mutex::new(Vec::new()),
        deleted_files: std::sync::Mutex::new(Vec::new()),
    });
    let mgr = SessionManager::new(&test_config(), Some(mock.clone()), None, Default::default());
    let result = mgr.run_consistency_check().await.unwrap();
    assert_eq!(result.deleted_orphaned_records, 0);
    assert_eq!(result.deleted_orphaned_files, 2);
    let deleted = mock.deleted_files.lock().unwrap();
    assert_eq!(deleted.len(), 2);
}

/// Verify that both orphan types are cleaned in a single check.
#[tokio::test]
async fn test_consistency_check_cleans_both_orphan_types() {
    let mock = Arc::new(BidirectionalMock {
        orphaned_records: 2,
        orphaned_files: 3,
        deleted_records: std::sync::Mutex::new(Vec::new()),
        deleted_files: std::sync::Mutex::new(Vec::new()),
    });
    let mgr = SessionManager::new(&test_config(), Some(mock.clone()), None, Default::default());
    let result = mgr.run_consistency_check().await.unwrap();
    assert_eq!(result.deleted_orphaned_records, 2);
    assert_eq!(result.deleted_orphaned_files, 3);
}

/// Verify that periodic consistency check can be spawned without panicking.
#[tokio::test]
async fn test_spawn_periodic_consistency_check() {
    let mock = Arc::new(CleanConsistencyMock);
    let mgr = Arc::new(SessionManager::new(
        &test_config(),
        Some(mock),
        None,
        Default::default(),
    ));
    // Should not panic; spawns a background task.
    mgr.spawn_periodic_consistency_check(std::time::Duration::from_secs(3600));
    // Give the task a moment to start (it skips the first tick).
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    // No assertion needed — if it panicked, the test would fail.
}
