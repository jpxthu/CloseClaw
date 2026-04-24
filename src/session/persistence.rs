//! Core persistence data structures and service trait
//!
//! Defines the core [`SessionCheckpoint`] structure and [`PersistenceService`] trait
//! for implementing pluggable storage backends.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

/// Session Checkpoint — 用于持久化恢复的核心数据结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCheckpoint {
    /// Session 唯一标识
    pub session_id: String,
    /// 最后一条持久化消息的 ID（平台相关）
    pub last_message_id: Option<String>,
    /// 当前推理模式状态
    pub mode_state: ReasoningModeState,
    /// 中间状态消息（尚未最终确认）
    pub pending_messages: Vec<PendingMessage>,
    /// 当前模式
    pub mode: ReasoningMode,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 最后更新时间
    pub updated_at: DateTime<Utc>,
    /// TTL（秒），0 表示不过期
    pub ttl_seconds: u64,
    /// 会话状态
    pub status: SessionStatus,
    /// 最后一条消息的时间
    pub last_message_at: Option<DateTime<Utc>>,
    /// 消息计数
    pub message_count: u64,
    /// 渠道标识
    pub channel: Option<String>,
    /// 聊天 ID
    pub chat_id: Option<String>,
}

impl SessionCheckpoint {
    /// Creates a new SessionCheckpoint with the current timestamp
    pub fn new(session_id: String) -> Self {
        let now = Utc::now();
        Self {
            session_id,
            last_message_id: None,
            mode_state: ReasoningModeState::default(),
            pending_messages: Vec::new(),
            mode: ReasoningMode::Direct,
            created_at: now,
            updated_at: now,
            ttl_seconds: 604800, // 7 days default
            status: SessionStatus::default(),
            last_message_at: None,
            message_count: 0,
            channel: None,
            chat_id: None,
        }
    }

    /// Update the last message ID
    pub fn with_last_message_id(mut self, message_id: Option<String>) -> Self {
        self.last_message_id = message_id;
        self
    }

    /// Update the mode
    pub fn with_mode(mut self, mode: ReasoningMode) -> Self {
        self.mode = mode;
        self
    }

    /// Update the mode state
    pub fn with_mode_state(mut self, state: ReasoningModeState) -> Self {
        self.mode_state = state;
        self
    }

    /// Add a pending message
    pub fn add_pending_message(mut self, msg: PendingMessage) -> Self {
        self.pending_messages.push(msg);
        self
    }

    /// Set TTL in seconds
    pub fn with_ttl(mut self, ttl: u64) -> Self {
        self.ttl_seconds = ttl;
        self
    }

    /// Update the session status
    pub fn with_status(mut self, status: SessionStatus) -> Self {
        self.status = status;
        self
    }

    /// Update the last message timestamp
    pub fn with_last_message_at(mut self, at: DateTime<Utc>) -> Self {
        self.last_message_at = Some(at);
        self
    }

    /// Update the message count
    pub fn with_message_count(mut self, count: u64) -> Self {
        self.message_count = count;
        self
    }

    /// Update the channel
    pub fn with_channel(mut self, channel: String) -> Self {
        self.channel = Some(channel);
        self
    }

    /// Update the chat ID
    pub fn with_chat_id(mut self, chat_id: String) -> Self {
        self.chat_id = Some(chat_id);
        self
    }

    /// Touch the updated_at timestamp
    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }
}

/// Reasoning Mode State — 推理模式的状态
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReasoningModeState {
    /// 当前步骤编号（1-indexed）
    pub current_step: u32,
    /// 总步骤数
    pub total_steps: u32,
    /// 各步骤的输出内容
    pub step_messages: Vec<String>,
    /// 是否完成
    pub is_complete: bool,
}

impl ReasoningModeState {
    /// Start a new reasoning step
    pub fn start_step(&mut self, total_steps: u32) {
        self.current_step += 1;
        self.total_steps = total_steps;
        self.is_complete = false;
    }

    /// Add a step message
    pub fn add_step_message(&mut self, message: String) {
        self.step_messages.push(message);
    }

    /// Mark reasoning as complete
    pub fn complete(&mut self) {
        self.is_complete = true;
    }
}

/// Pending Message — 未最终确认的消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingMessage {
    /// 消息 ID
    pub message_id: String,
    /// 消息内容
    pub content: String,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 是否已发送
    pub sent: bool,
}

impl PendingMessage {
    /// Create a new pending message
    pub fn new(message_id: String, content: String) -> Self {
        Self {
            message_id,
            content,
            created_at: Utc::now(),
            sent: false,
        }
    }

    /// Mark the message as sent
    pub fn mark_sent(&mut self) {
        self.sent = true;
    }
}

/// Reasoning Mode — 推理模式枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningMode {
    /// 直接回答模式
    Direct,
    /// 规划模式（先展示思考框架）
    Plan,
    /// 流式输出模式
    Stream,
    /// 隐藏思考过程模式
    Hidden,
}

impl Default for ReasoningMode {
    fn default() -> Self {
        ReasoningMode::Direct
    }
}

impl std::fmt::Display for ReasoningMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReasoningMode::Direct => write!(f, "direct"),
            ReasoningMode::Plan => write!(f, "plan"),
            ReasoningMode::Stream => write!(f, "stream"),
            ReasoningMode::Hidden => write!(f, "hidden"),
        }
    }
}

/// Session Status — 会话生命周期状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    /// 活跃状态
    Active,
    /// 已归档状态
    Archived,
}

impl Default for SessionStatus {
    fn default() -> Self {
        SessionStatus::Active
    }
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Active => write!(f, "active"),
            SessionStatus::Archived => write!(f, "archived"),
        }
    }
}

/// Agent Role — 智能体角色枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentRole {
    /// 主智能体
    MainAgent,
    /// 分身智能体
    SubAgent,
}

/// Persistence errors
#[derive(Error, Debug)]
pub enum PersistenceError {
    #[error("Redis error: {0}")]
    Redis(String),
    #[error("PostgreSQL error: {0}")]
    Postgres(String),
    #[error("SQLite error: {0}")]
    Sqlite(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Checkpoint not found for session: {0}")]
    NotFound(String),
    #[error("Lock error: {0}")]
    Lock(String),
}

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

    /// 删除 Checkpoint
    async fn delete_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError>;

    /// 列出所有活跃 Session 的 Checkpoint
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError>;

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

    /// 使给定 session 的本地缓存失效（无实际操作，直接返回 Ok）。
    async fn invalidate_session(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
}

/// Checkpoint 管理器 — 负责保存和恢复 Session 状态
pub struct CheckpointManager<S: PersistenceService> {
    storage: Arc<S>,
    /// 本地缓存（减少对存储的访问）
    local_cache: RwLock<HashMap<String, SessionCheckpoint>>,
}

impl<S: PersistenceService + 'static> CheckpointManager<S> {
    /// Create a new CheckpointManager with the given storage
    pub fn new(storage: Arc<S>) -> Self {
        Self {
            storage,
            local_cache: RwLock::new(HashMap::new()),
        }
    }

    /// Get a reference to the underlying storage
    pub fn storage(&self) -> &S {
        &*self.storage
    }

    /// 保存 Checkpoint（异步写入，不阻塞主流程）
    pub async fn save(&self, checkpoint: SessionCheckpoint) -> Result<(), PersistenceError> {
        let session_id = checkpoint.session_id.clone();

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
    pub async fn save_sync(&self, checkpoint: SessionCheckpoint) -> Result<(), PersistenceError> {
        let session_id = checkpoint.session_id.clone();

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
