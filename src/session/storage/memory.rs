//! Memory storage backend for session persistence
//!
//! This backend stores checkpoints in memory using a `HashMap` protected by an `RwLock`.
//! Suitable for testing and single-instance deployments.

use crate::session::persistence::{PersistenceError, PersistenceService, SessionCheckpoint};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::RwLock;

/// Memory storage backend (only for testing)
#[derive(Debug)]
pub struct MemoryStorage {
    checkpoints: RwLock<HashMap<String, SessionCheckpoint>>,
    archived: RwLock<HashMap<String, SessionCheckpoint>>,
}

impl MemoryStorage {
    /// Create a new MemoryStorage instance
    pub fn new() -> Self {
        Self {
            checkpoints: RwLock::new(HashMap::new()),
            archived: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PersistenceService for MemoryStorage {
    async fn save_checkpoint(
        &self,
        checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        let mut checkpoints = self
            .checkpoints
            .write()
            .map_err(|_| PersistenceError::Lock("RwLock write failed".to_string()))?;
        checkpoints.insert(checkpoint.session_id.clone(), checkpoint.clone());
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        let checkpoints = self
            .checkpoints
            .read()
            .map_err(|_| PersistenceError::Lock("RwLock read failed".to_string()))?;
        Ok(checkpoints.get(session_id).cloned())
    }

    async fn delete_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError> {
        let mut checkpoints = self
            .checkpoints
            .write()
            .map_err(|_| PersistenceError::Lock("RwLock write failed".to_string()))?;
        checkpoints.remove(session_id);
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        let checkpoints = self
            .checkpoints
            .read()
            .map_err(|_| PersistenceError::Lock("RwLock read failed".to_string()))?;
        Ok(checkpoints.keys().cloned().collect())
    }

    async fn archive_checkpoint(
        &self,
        checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        let mut archived = self
            .archived
            .write()
            .map_err(|_| PersistenceError::Lock("RwLock write failed".to_string()))?;
        archived.insert(checkpoint.session_id.clone(), checkpoint.clone());
        Ok(())
    }

    async fn restore_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        let archived = self
            .archived
            .read()
            .map_err(|_| PersistenceError::Lock("RwLock read failed".to_string()))?;
        Ok(archived.get(session_id).cloned())
    }

    async fn purge_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError> {
        let mut archived = self
            .archived
            .write()
            .map_err(|_| PersistenceError::Lock("RwLock write failed".to_string()))?;
        archived.remove(session_id);
        Ok(())
    }

    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        let archived = self
            .archived
            .read()
            .map_err(|_| PersistenceError::Lock("RwLock read failed".to_string()))?;
        Ok(archived.keys().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::persistence::{ReasoningMode, ReasoningModeState, SessionStatus};
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
            status: SessionStatus::Active,
            last_message_at: None,
            message_count: 0,
            channel: None,
            chat_id: None,
        }
    }

    #[tokio::test]
    async fn test_memory_storage_save_and_load() {
        let storage = MemoryStorage::new();
        let checkpoint = create_test_checkpoint("session1");

        storage.save_checkpoint(&checkpoint).await.unwrap();

        let loaded = storage.load_checkpoint("session1").await.unwrap();
        assert!(loaded.is_some());

        let loaded = loaded.unwrap();
        assert_eq!(loaded.session_id, "session1");
        assert_eq!(loaded.last_message_id, Some("msg123".to_string()));
        assert_eq!(loaded.mode, ReasoningMode::Plan);
    }

    #[tokio::test]
    async fn test_memory_storage_load_none() {
        let storage = MemoryStorage::new();

        let loaded = storage.load_checkpoint("nonexistent").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_memory_storage_delete() {
        let storage = MemoryStorage::new();
        let checkpoint = create_test_checkpoint("session2");

        storage.save_checkpoint(&checkpoint).await.unwrap();
        storage.delete_checkpoint("session2").await.unwrap();

        let loaded = storage.load_checkpoint("session2").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_memory_storage_list_active_sessions() {
        let storage = MemoryStorage::new();

        storage
            .save_checkpoint(&create_test_checkpoint("session_a"))
            .await
            .unwrap();
        storage
            .save_checkpoint(&create_test_checkpoint("session_b"))
            .await
            .unwrap();

        let sessions = storage.list_active_sessions().await.unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(sessions.contains(&"session_a".to_string()));
        assert!(sessions.contains(&"session_b".to_string()));
    }

    #[tokio::test]
    async fn test_memory_storage_overwrite() {
        let storage = MemoryStorage::new();
        let checkpoint1 = create_test_checkpoint("session3");
        storage.save_checkpoint(&checkpoint1).await.unwrap();

        let mut checkpoint2 = create_test_checkpoint("session3");
        checkpoint2.last_message_id = Some("msg456".to_string());
        storage.save_checkpoint(&checkpoint2).await.unwrap();

        let loaded = storage.load_checkpoint("session3").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().last_message_id, Some("msg456".to_string()));
    }
}
