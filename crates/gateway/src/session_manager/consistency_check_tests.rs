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
