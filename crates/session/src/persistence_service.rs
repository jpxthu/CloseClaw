//! Persistence service trait.

use async_trait::async_trait;

use crate::persistence::{
    AgentRole, ConsistencyCheckResult, DreamingStatus, PersistenceError, SessionCheckpoint,
};

/// 持久化服务接口
#[async_trait]
pub trait PersistenceService: Send + Sync {
    /// 保存 Checkpoint
    async fn save_checkpoint(&self, checkpoint: &SessionCheckpoint)
        -> Result<(), PersistenceError>;

    /// 加载 Checkpoint
    async fn load_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError>;

    /// 加载已归档的 Checkpoint
    async fn load_archived_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }

    /// 删除 Checkpoint
    async fn delete_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError>;

    /// 列出所有活跃 Session 的 Checkpoint
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError>;

    /// 查找与给定 routing fields 匹配的 active session。
    ///
    /// 用于创建新 session 前的防御性双重确认（SQLite 双重确认）。
    /// 当 `account_id` 为 `None` 时，匹配数据库中 `account_id IS NULL` 的记录。
    ///
    /// 返回匹配的 session_id，若无匹配返回 `Ok(None)`。
    async fn find_active_session_by_routing(
        &self,
        _account_id: Option<&str>,
        _channel: &str,
        _sender_id: &str,
        _peer_id: &str,
    ) -> Result<Option<String>, PersistenceError> {
        Ok(None)
    }

    /// 查找与给定 routing fields 匹配的 migrating session。
    ///
    /// 用于 registry miss 路径：当 key_registry 未命中且活跃会话也不存在时，
    /// 通过路由字段查询正在迁移中的会话，等待归档完成后恢复。
    /// 当 `account_id` 为 `None` 时，匹配数据库中 `account_id IS NULL` 的记录。
    ///
    /// 返回匹配的 session_id，若无匹配返回 `Ok(None)`。
    async fn find_migrating_session_by_routing(
        &self,
        _account_id: Option<&str>,
        _channel: &str,
        _sender_id: &str,
        _peer_id: &str,
    ) -> Result<Option<String>, PersistenceError> {
        Ok(None)
    }

    /// 查找与给定 routing fields 匹配的 archived session。
    ///
    /// 用于归档恢复路径：当 key_registry 未命中且活跃会话也不存在时，
    /// 通过路由字段查询归档会话。返回 `last_message_at` 最新的那条。
    /// 当 `account_id` 为 `None` 时，匹配数据库中 `account_id IS NULL` 的记录。
    ///
    /// 返回匹配的 session_id，若无匹配返回 `Ok(None)`。
    async fn find_archived_session_by_routing(
        &self,
        _account_id: Option<&str>,
        _channel: &str,
        _sender_id: &str,
        _peer_id: &str,
    ) -> Result<Option<String>, PersistenceError> {
        Ok(None)
    }

    /// 归档 Checkpoint
    async fn archive_checkpoint(
        &self,
        _checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        Err(PersistenceError::NotFound(_checkpoint.session_id.clone()))
    }

    /// 恢复已归档的 Checkpoint
    async fn restore_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Err(PersistenceError::NotFound(session_id.to_string()))
    }

    /// 永久删除已归档的 Checkpoint
    async fn purge_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError> {
        Err(PersistenceError::NotFound(session_id.to_string()))
    }

    /// 列出已归档的 Session
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    /// 列出正在迁移中的 Session（归档中断的崩溃安全中间状态）
    async fn list_migrating_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    /// 使给定 session 的本地缓存失效（无实际操作，直接返回 Ok）。
    async fn invalidate_session(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    /// Force a WAL checkpoint to ensure all pending writes are flushed to disk.
    ///
    /// The default implementation is a no-op (returns `Ok(())`). Concrete
    /// storage backends should override this to issue a `PRAGMA wal_checkpoint`
    /// or equivalent.
    async fn sync(&self) -> Result<(), PersistenceError> {
        Ok(())
    }

    /// List IDs of active sessions for a specific agent/role idle for at least
    /// `idle_minutes`.
    async fn list_idle_sessions_for_agent(
        &self,
        _agent_id: &str,
        _role: AgentRole,
        _idle_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    /// List IDs of archived sessions for a specific agent/role past their purge
    /// window.
    async fn list_expired_archived_sessions_for_agent(
        &self,
        _agent_id: &str,
        _role: AgentRole,
        _purge_after_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    /// 列出指定 session 的所有直接子 session（parent_session_id = session_id）
    async fn list_children_sessions(
        &self,
        _parent_session_id: &str,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    /// 列出已归档且尚未被 memory-miner 挖掘的 session ID
    async fn list_archived_unmined_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    /// 列出已挖掘（mined=true）但 dreaming 未完成的 session ID
    async fn list_mined_undreamt_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    /// 标记指定 session 已被 memory-miner 挖掘
    async fn mark_mined(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    /// 更新指定 session 的 dreaming 状态
    async fn update_dreaming_status(
        &self,
        _session_id: &str,
        _status: DreamingStatus,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    /// Explicitly close the storage backend and release resources.
    ///
    /// Called during Phase 6 of daemon shutdown. The default implementation
    /// is a no-op (returns `Ok(())`). Concrete storage backends should
    /// override this to close persistent connections or file handles.
    async fn close(&self) -> Result<(), PersistenceError> {
        Ok(())
    }

    /// Run a bidirectional consistency check between SQLite and the file system.
    ///
    /// - SQLite → File system: records whose transcript files are missing → deleted.
    /// - File system → SQLite: orphan transcript files with no SQLite record → deleted.
    ///
    /// The default implementation is a no-op. Concrete storage backends
    /// (e.g. `SqliteStorage`) should override this to perform the actual check.
    async fn run_consistency_check(&self) -> Result<ConsistencyCheckResult, PersistenceError> {
        Ok(ConsistencyCheckResult::default())
    }

    /// Run an incremental bidirectional consistency check since the given
    /// Unix epoch seconds (`since`).
    ///
    /// - SQLite → File system: only active records with `last_message_at > since`.
    /// - File system → SQLite: only transcript files with `mtime > since`.
    ///
    /// The default implementation delegates to the full `run_consistency_check`
    /// (ignoring `since`), preserving backward compatibility.
    async fn run_incremental_consistency_check(
        &self,
        _since: i64,
    ) -> Result<ConsistencyCheckResult, PersistenceError> {
        self.run_consistency_check().await
    }
}
