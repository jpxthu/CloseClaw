//! Checkpoint Manager — manages session checkpoint save/restore with local cache
//!
//! Provides optional identity auto-filling: if created with [`new_with_identity`],
//! every checkpoint saved through this manager will have its `agent_id` and `role`
//! fields automatically set (overriding any existing values).

use super::{AgentRole, PersistenceError, PersistenceService, SessionCheckpoint};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Checkpoint Manager — saves and restores session state with optional identity
pub struct CheckpointManager<S: PersistenceService> {
    storage: Arc<S>,
    /// Local cache to reduce storage access
    local_cache: RwLock<HashMap<String, SessionCheckpoint>>,
    /// Identity fields for auto-filling (None when not configured)
    identity: Option<(String, AgentRole)>,
}

impl<S: PersistenceService + 'static> CheckpointManager<S> {
    /// Create a new CheckpointManager with the given storage
    pub fn new(storage: Arc<S>) -> Self {
        Self {
            storage,
            local_cache: RwLock::new(HashMap::new()),
            identity: Some((String::new(), AgentRole::MainAgent)),
        }
    }

    /// Create a CheckpointManager with identity for automatic filling
    ///
    /// Every checkpoint saved through this manager will have its `agent_id`
    /// and `role` fields automatically set to the given values.
    pub fn new_with_identity(storage: Arc<S>, agent_id: String, role: AgentRole) -> Self {
        Self {
            storage,
            local_cache: RwLock::new(HashMap::new()),
            identity: Some((agent_id, role)),
        }
    }

    /// Get a reference to the underlying storage
    pub fn storage(&self) -> &S {
        &*self.storage
    }

    /// Apply identity fields to a checkpoint (if identity is configured)
    fn apply_identity(&self, mut checkpoint: SessionCheckpoint) -> SessionCheckpoint {
        if let Some((ref agent_id, ref role)) = self.identity {
            checkpoint.agent_id = Some(agent_id.clone());
            checkpoint.role = Some(*role);
        }
        checkpoint
    }

    /// Save checkpoint (async write, does not block main flow)
    pub async fn save(&self, checkpoint: SessionCheckpoint) -> Result<(), PersistenceError> {
        let checkpoint = self.apply_identity(checkpoint);
        let session_id = checkpoint.session_id.clone();

        {
            let mut cache = self.local_cache.write().await;
            cache.insert(session_id.clone(), checkpoint.clone());
        }

        let storage = Arc::clone(&self.storage);
        tokio::spawn(async move {
            if let Err(e) = storage.save_checkpoint(&checkpoint).await {
                tracing::error!(session_id = %checkpoint.session_id, "Failed to save checkpoint: {}", e);
            }
        });

        Ok(())
    }

    /// Save checkpoint (sync write, used when gateway shuts down)
    pub async fn save_sync(&self, checkpoint: SessionCheckpoint) -> Result<(), PersistenceError> {
        let checkpoint = self.apply_identity(checkpoint);
        let session_id = checkpoint.session_id.clone();

        {
            let mut cache = self.local_cache.write().await;
            cache.insert(session_id.clone(), checkpoint.clone());
        }

        self.storage.save_checkpoint(&checkpoint).await
    }

    /// Load checkpoint (local cache first)
    pub async fn load(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        {
            let cache = self.local_cache.read().await;
            if let Some(cp) = cache.get(session_id) {
                return Ok(Some(cp.clone()));
            }
        }

        let cp = self.storage.load_checkpoint(session_id).await?;

        if let Some(ref checkpoint) = cp {
            let mut cache = self.local_cache.write().await;
            cache.insert(session_id.to_string(), checkpoint.clone());
        }

        Ok(cp)
    }

    /// Delete checkpoint
    pub async fn delete(&self, session_id: &str) -> Result<(), PersistenceError> {
        {
            let mut cache = self.local_cache.write().await;
            cache.remove(session_id);
        }

        self.storage.delete_checkpoint(session_id).await
    }

    /// Clear local cache
    pub async fn clear_cache(&self) {
        let mut cache = self.local_cache.write().await;
        cache.clear();
    }

    /// Get cached session ids
    pub async fn cached_session_ids(&self) -> Vec<String> {
        let cache = self.local_cache.read().await;
        cache.keys().cloned().collect()
    }

    /// Archive checkpoint
    pub async fn archive(&self, checkpoint: SessionCheckpoint) -> Result<(), PersistenceError> {
        let checkpoint = self.apply_identity(checkpoint);
        self.storage.archive_checkpoint(&checkpoint).await?;
        {
            let mut cache = self.local_cache.write().await;
            cache.remove(&checkpoint.session_id);
        }
        self.storage
            .delete_checkpoint(&checkpoint.session_id)
            .await?;
        Ok(())
    }

    /// Restore archived checkpoint
    pub async fn restore(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        let cp = self.storage.restore_checkpoint(session_id).await?;
        if let Some(ref checkpoint) = cp {
            self.storage.save_checkpoint(checkpoint).await?;
            self.storage.purge_checkpoint(session_id).await?;
            let mut cache = self.local_cache.write().await;
            cache.insert(session_id.to_string(), checkpoint.clone());
        }
        Ok(cp)
    }

    /// Permanently delete archived checkpoint
    pub async fn purge(&self, session_id: &str) -> Result<(), PersistenceError> {
        self.storage.purge_checkpoint(session_id).await
    }

    /// List archived session ids
    pub async fn archived_session_ids(&self) -> Result<Vec<String>, PersistenceError> {
        self.storage.list_archived_sessions().await
    }
}
