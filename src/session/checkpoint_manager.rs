//! Checkpoint Manager — 负责保存和恢复 Session 状态
//!
//! 将 [`CheckpointManager`] 从 [`super::persistence`] 模块中拆分出来，降低 persistence.rs 的行数。

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::persistence::{AgentRole, PersistenceError, PersistenceService, SessionCheckpoint};

/// Checkpoint 管理器 — 负责保存和恢复 Session 状态
pub struct CheckpointManager<S: PersistenceService> {
    storage: Arc<S>,
    /// 本地缓存（减少对存储的访问）
    local_cache: RwLock<HashMap<String, SessionCheckpoint>>,
    /// Identity: agent_id
    agent_id: Option<String>,
    /// Identity: role
    role: Option<AgentRole>,
}

impl<S: PersistenceService + 'static> CheckpointManager<S> {
    /// Create a new CheckpointManager with the given storage
    pub fn new(storage: Arc<S>) -> Self {
        Self {
            storage,
            local_cache: RwLock::new(HashMap::new()),
            agent_id: Some(String::new()),
            role: Some(AgentRole::MainAgent),
        }
    }

    /// Create a new CheckpointManager with explicit identity
    pub fn new_with_identity(storage: Arc<S>, agent_id: String, role: AgentRole) -> Self {
        Self {
            storage,
            local_cache: RwLock::new(HashMap::new()),
            agent_id: Some(agent_id),
            role: Some(role),
        }
    }

    /// Get a reference to the underlying storage
    pub fn storage(&self) -> &S {
        &*self.storage
    }

    /// 保存 Checkpoint（异步写入，不阻塞主流程）
    pub async fn save(&self, mut checkpoint: SessionCheckpoint) -> Result<(), PersistenceError> {
        let session_id = checkpoint.session_id.clone();

        // Inject identity into checkpoint (manager's identity always overrides)
        if let Some(ref agent_id) = self.agent_id {
            checkpoint.agent_id = Some(agent_id.clone());
        }
        if let Some(ref role) = self.role {
            checkpoint.role = Some(*role);
        }

        // 先更新本地缓存
        {
            let mut cache = self.local_cache.write().await;
            cache.insert(session_id.clone(), checkpoint.clone());
        }

        // 异步保存到存储后端
        let storage = Arc::clone(&self.storage);
        tokio::spawn(async move {
            if let Err(e) = storage.save_checkpoint(&checkpoint).await {
                tracing::error!(session_id = %checkpoint.session_id, "Failed to save checkpoint: {}", e);
            }
        });

        Ok(())
    }

    /// 保存 Checkpoint（同步写入，用于网关关闭时）
    pub async fn save_sync(
        &self,
        mut checkpoint: SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        let session_id = checkpoint.session_id.clone();

        // Inject identity into checkpoint (manager's identity always overrides)
        if let Some(ref agent_id) = self.agent_id {
            checkpoint.agent_id = Some(agent_id.clone());
        }
        if let Some(ref role) = self.role {
            checkpoint.role = Some(*role);
        }

        // 先更新本地缓存
        {
            let mut cache = self.local_cache.write().await;
            cache.insert(session_id.clone(), checkpoint.clone());
        }

        // 同步保存到存储后端
        self.storage.save_checkpoint(&checkpoint).await
    }

    /// 加载 Checkpoint（优先本地缓存）
    pub async fn load(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        // 先查本地缓存
        {
            let cache = self.local_cache.read().await;
            if let Some(cp) = cache.get(session_id) {
                return Ok(Some(cp.clone()));
            }
        }

        // 缓存未命中，从存储加载
        let cp = self.storage.load_checkpoint(session_id).await?;

        if let Some(ref checkpoint) = cp {
            // 更新本地缓存
            let mut cache = self.local_cache.write().await;
            cache.insert(session_id.to_string(), checkpoint.clone());
        }

        Ok(cp)
    }

    /// 删除 Checkpoint
    pub async fn delete(&self, session_id: &str) -> Result<(), PersistenceError> {
        // 删除本地缓存
        {
            let mut cache = self.local_cache.write().await;
            cache.remove(session_id);
        }

        // 删除存储中的数据
        self.storage.delete_checkpoint(session_id).await
    }

    /// 清空本地缓存
    pub async fn clear_cache(&self) {
        let mut cache = self.local_cache.write().await;
        cache.clear();
    }

    /// 获取缓存中所有 session_id
    pub async fn cached_session_ids(&self) -> Vec<String> {
        let cache = self.local_cache.read().await;
        cache.keys().cloned().collect()
    }

    /// 归档 Checkpoint
    pub async fn archive(&self, checkpoint: SessionCheckpoint) -> Result<(), PersistenceError> {
        self.storage.archive_checkpoint(&checkpoint).await?;
        // 从本地缓存和活跃存储中移除
        {
            let mut cache = self.local_cache.write().await;
            cache.remove(&checkpoint.session_id);
        }
        self.storage
            .delete_checkpoint(&checkpoint.session_id)
            .await?;
        Ok(())
    }

    /// 恢复已归档的 Checkpoint
    pub async fn restore(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        let cp = self.storage.restore_checkpoint(session_id).await?;
        if let Some(ref checkpoint) = cp {
            // 恢复到活跃存储
            self.storage.save_checkpoint(checkpoint).await?;
            self.storage.purge_checkpoint(session_id).await?;
            // 更新本地缓存
            let mut cache = self.local_cache.write().await;
            cache.insert(session_id.to_string(), checkpoint.clone());
        }
        Ok(cp)
    }

    /// 永久删除已归档的 Checkpoint
    pub async fn purge(&self, session_id: &str) -> Result<(), PersistenceError> {
        self.storage.purge_checkpoint(session_id).await
    }

    /// 列出已归档的 Session ID
    pub async fn archived_session_ids(&self) -> Result<Vec<String>, PersistenceError> {
        self.storage.list_archived_sessions().await
    }
}
