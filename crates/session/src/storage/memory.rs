//! Memory storage backend for session persistence
//!
//! This backend stores checkpoints in memory using a `HashMap` protected by an `RwLock`.
//! Suitable for testing and single-instance deployments.

use crate::persistence::{DreamingStatus, PersistenceError, PersistenceService, SessionCheckpoint};
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

    /// Insert a session ID into the archived list without checkpoint data.
    /// Used in tests to simulate a "checkpoint not found" scenario for archived sessions.
    #[cfg(test)]
    pub async fn insert_archived_id(&self, session_id: &str) {
        let mut archived = self
            .archived
            .write()
            .map_err(|_| PersistenceError::Lock("RwLock write failed".to_string()))
            .unwrap();
        // Use a dummy checkpoint with the given session_id so it appears in
        // list_archived_sessions. The dummy will be removed by restore_checkpoint,
        // but load_checkpoint will not find it in active, triggering the not-found path.
        archived.insert(
            session_id.to_string(),
            SessionCheckpoint::new(session_id.to_string()),
        );
    }

    /// Remove a session from the active checkpoint map.
    /// Used in tests to simulate an archived-only session (not in active list).
    #[cfg(test)]
    pub async fn remove_active(&self, session_id: &str) {
        let mut checkpoints = self
            .checkpoints
            .write()
            .map_err(|_| PersistenceError::Lock("RwLock write failed".to_string()))
            .unwrap();
        checkpoints.remove(session_id);
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

    async fn load_archived_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        let archived = self
            .archived
            .read()
            .map_err(|_| PersistenceError::Lock("RwLock read failed".to_string()))?;
        Ok(archived.get(session_id).cloned())
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

    async fn find_active_session_by_routing(
        &self,
        account_id: Option<&str>,
        channel: &str,
        sender_id: &str,
        peer_id: &str,
    ) -> Result<Option<String>, PersistenceError> {
        let checkpoints = self
            .checkpoints
            .read()
            .map_err(|_| PersistenceError::Lock("RwLock read failed".to_string()))?;
        for (id, cp) in checkpoints.iter() {
            if cp.status != crate::persistence::SessionStatus::Active {
                continue;
            }
            let cp_channel = cp.platform.as_deref().unwrap_or("");
            let cp_sender = cp.sender_id.as_deref().unwrap_or("");
            let cp_peer = cp.peer_id.as_deref().unwrap_or("");
            let cp_account = cp.account_id.as_deref();
            if cp_channel == channel
                && cp_sender == sender_id
                && cp_peer == peer_id
                && cp_account == account_id
            {
                return Ok(Some(id.clone()));
            }
        }
        Ok(None)
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
        let mut archived = self
            .archived
            .write()
            .map_err(|_| PersistenceError::Lock("RwLock write failed".to_string()))?;
        match archived.remove(session_id) {
            Some(cp) => {
                let mut checkpoints = self
                    .checkpoints
                    .write()
                    .map_err(|_| PersistenceError::Lock("RwLock write failed".to_string()))?;
                checkpoints.insert(session_id.to_string(), cp.clone());
                Ok(Some(cp))
            }
            None => Ok(None),
        }
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

    async fn list_children_sessions(
        &self,
        parent_session_id: &str,
    ) -> Result<Vec<String>, PersistenceError> {
        let checkpoints = self
            .checkpoints
            .read()
            .map_err(|_| PersistenceError::Lock("RwLock read failed".to_string()))?;
        let children: Vec<String> = checkpoints
            .values()
            .filter(|cp| cp.parent_session_id.as_deref() == Some(parent_session_id))
            .map(|cp| cp.session_id.clone())
            .collect();
        Ok(children)
    }

    async fn list_archived_unmined_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        let archived = self
            .archived
            .read()
            .map_err(|_| PersistenceError::Lock("RwLock read failed".to_string()))?;
        let unmined: Vec<String> = archived
            .values()
            .filter(|cp| !cp.mined)
            .map(|cp| cp.session_id.clone())
            .collect();
        Ok(unmined)
    }

    async fn list_mined_undreamt_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        let checkpoints = self
            .checkpoints
            .read()
            .map_err(|_| PersistenceError::Lock("RwLock read failed".to_string()))?;
        let archived = self
            .archived
            .read()
            .map_err(|_| PersistenceError::Lock("RwLock read failed".to_string()))?;
        let mut result = Vec::new();
        for cp in checkpoints.values().chain(archived.values()) {
            if cp.mined && cp.dreaming_status != DreamingStatus::Completed {
                result.push(cp.session_id.clone());
            }
        }
        result.sort();
        result.dedup();
        Ok(result)
    }

    async fn mark_mined(&self, session_id: &str) -> Result<(), PersistenceError> {
        let now = chrono::Utc::now().timestamp();
        let mut checkpoints = self
            .checkpoints
            .write()
            .map_err(|_| PersistenceError::Lock("RwLock write failed".to_string()))?;
        if let Some(cp) = checkpoints.get_mut(session_id) {
            cp.mined = true;
            cp.mined_at = Some(now);
            return Ok(());
        }
        let mut archived = self
            .archived
            .write()
            .map_err(|_| PersistenceError::Lock("RwLock write failed".to_string()))?;
        if let Some(cp) = archived.get_mut(session_id) {
            cp.mined = true;
            cp.mined_at = Some(now);
            return Ok(());
        }
        Ok(())
    }

    async fn update_dreaming_status(
        &self,
        session_id: &str,
        status: DreamingStatus,
    ) -> Result<(), PersistenceError> {
        let mut checkpoints = self
            .checkpoints
            .write()
            .map_err(|_| PersistenceError::Lock("RwLock write failed".to_string()))?;
        if let Some(cp) = checkpoints.get_mut(session_id) {
            cp.dreaming_status = status;
            return Ok(());
        }
        let mut archived = self
            .archived
            .write()
            .map_err(|_| PersistenceError::Lock("RwLock write failed".to_string()))?;
        if let Some(cp) = archived.get_mut(session_id) {
            cp.dreaming_status = status;
            return Ok(());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::{
        DreamingStatus, ReasoningLevel, ReasoningMode, ReasoningModeState, SessionMode,
        SessionStatus,
    };
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
            outbound_pending: Vec::new(),
            reasoning_mode: ReasoningMode::Plan,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            ttl_seconds: 604800,
            status: SessionStatus::Active,
            last_message_at: None,
            message_count: 0,
            platform: None,
            peer_id: None,
            account_id: None,
            agent_id: None,
            role: None,
            reasoning_level: ReasoningLevel::default(),
            system_appends: Vec::new(),
            thread_id: None,
            sender_id: None,
            parent_session_id: None,
            depth: 0,
            effective_max_spawn_depth: None,
            mined: false,
            mined_at: None,
            dreaming_status: DreamingStatus::default(),
            pending_operations: Vec::new(),
            recovery_notification: None,
            pending_tool_failures: Vec::new(),
            verbosity_level: closeclaw_common::VerbosityLevel::default(),
            plan_state: None,
            progress_tool_calls: Vec::new(),
            approval_tool_calls: Vec::new(),
            plan_references: Vec::new(),
            session_mode: SessionMode::default(),
            pending_messages: Vec::new(),
            label: None,
            communication_config: None,
            spawn_mode: None,
            snapshot_metas: Vec::new(),
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
        assert_eq!(loaded.reasoning_mode, ReasoningMode::Plan);
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

    // ── list_children_sessions tests ──────────────────────────────────────────

    #[tokio::test]
    async fn test_list_children_sessions_no_children() {
        let storage = MemoryStorage::new();
        let cp = create_test_checkpoint("parent-only");
        storage.save_checkpoint(&cp).await.unwrap();

        let children = storage.list_children_sessions("parent-only").await.unwrap();
        assert!(children.is_empty(), "no children should return empty vec");
    }

    #[tokio::test]
    async fn test_list_children_sessions_with_children() {
        let storage = MemoryStorage::new();

        // Parent
        let mut parent = create_test_checkpoint("parent");
        parent.parent_session_id = None;
        storage.save_checkpoint(&parent).await.unwrap();

        // Child 1
        let mut child1 = create_test_checkpoint("child1");
        child1.parent_session_id = Some("parent".to_string());
        storage.save_checkpoint(&child1).await.unwrap();

        // Child 2
        let mut child2 = create_test_checkpoint("child2");
        child2.parent_session_id = Some("parent".to_string());
        storage.save_checkpoint(&child2).await.unwrap();

        // Unrelated session
        let mut unrelated = create_test_checkpoint("unrelated");
        unrelated.parent_session_id = Some("other-parent".to_string());
        storage.save_checkpoint(&unrelated).await.unwrap();

        let mut children = storage.list_children_sessions("parent").await.unwrap();
        children.sort();
        assert_eq!(children, vec!["child1".to_string(), "child2".to_string()]);
    }

    #[tokio::test]
    async fn test_list_children_sessions_empty_storage() {
        let storage = MemoryStorage::new();
        let children = storage.list_children_sessions("nonexistent").await.unwrap();
        assert!(children.is_empty());
    }

    #[tokio::test]
    async fn test_list_children_sessions_after_delete() {
        let storage = MemoryStorage::new();

        let mut child = create_test_checkpoint("child-del");
        child.parent_session_id = Some("parent-del".to_string());
        storage.save_checkpoint(&child).await.unwrap();

        // Verify child exists
        let children = storage.list_children_sessions("parent-del").await.unwrap();
        assert_eq!(children, vec!["child-del".to_string()]);

        // Delete child
        storage.delete_checkpoint("child-del").await.unwrap();

        // Should be empty now
        let children = storage.list_children_sessions("parent-del").await.unwrap();
        assert!(children.is_empty());
    }

    #[tokio::test]
    async fn test_list_children_sessions_nested() {
        let storage = MemoryStorage::new();

        // root -> child1 -> grandchild
        let mut root = create_test_checkpoint("root");
        root.parent_session_id = None;
        storage.save_checkpoint(&root).await.unwrap();

        let mut child1 = create_test_checkpoint("child1");
        child1.parent_session_id = Some("root".to_string());
        storage.save_checkpoint(&child1).await.unwrap();

        let mut grandchild = create_test_checkpoint("grandchild");
        grandchild.parent_session_id = Some("child1".to_string());
        storage.save_checkpoint(&grandchild).await.unwrap();

        // root's direct children should only be child1, not grandchild
        let root_children = storage.list_children_sessions("root").await.unwrap();
        assert_eq!(root_children, vec!["child1".to_string()]);

        // child1's direct children should only be grandchild
        let child1_children = storage.list_children_sessions("child1").await.unwrap();
        assert_eq!(child1_children, vec!["grandchild".to_string()]);
    }
}
