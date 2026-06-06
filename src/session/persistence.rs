//! Core persistence data structures and service trait
//!
//! Defines the core [`SessionCheckpoint`] structure and [`PersistenceService`] trait
//! for implementing pluggable storage backends.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    /// 来源平台标识（原 channel）
    #[serde(default)]
    pub platform: Option<String>,
    /// 会话对端标识（原 chat_id）
    #[serde(default)]
    pub peer_id: Option<String>,
    pub agent_id: Option<String>,
    pub role: Option<AgentRole>,
    /// 推理深度等级
    pub reasoning_level: ReasoningLevel,
    /// Per-session 追加区内容（system prompt append section）
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为空 Vec）。
    #[serde(default)]
    pub system_appends: Vec<String>,
    /// 话题 ID（IM 渠道话题消息的线程标识）
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为 None）。
    #[serde(default)]
    pub thread_id: Option<String>,
    /// 租户标识（可选）
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为 None）。
    #[serde(default)]
    pub account_id: Option<String>,
    /// 消息发送者 ID（用于 session_key 重建）
    ///
    /// 存储原始消息的 `from` 字段，使得 `rebuild_key_registry` 在重启后
    /// 能用 `compute_session_key(PerChannelPeer)` 的格式
    /// `"{channel}:{from}:{to}"` 正确重建 key → session_id 映射。
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为 None）。
    #[serde(default)]
    pub sender_id: Option<String>,
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
            platform: None,
            peer_id: None,
            account_id: None,
            agent_id: None,
            role: None,
            reasoning_level: ReasoningLevel::default(),
            system_appends: Vec::new(),
            thread_id: None,
            sender_id: None,
        }
    }

    /// Update the agent_id
    pub fn with_agent_id(mut self, agent_id: String) -> Self {
        self.agent_id = Some(agent_id);
        self
    }
    /// Update the role
    pub fn with_role(mut self, role: AgentRole) -> Self {
        self.role = Some(role);
        self
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
    /// Set the pending messages list
    pub fn with_pending_messages(mut self, msgs: Vec<PendingMessage>) -> Self {
        self.pending_messages = msgs;
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
}

impl SessionCheckpoint {
    /// Update the message count
    pub fn with_message_count(mut self, count: u64) -> Self {
        self.message_count = count;
        self
    }
    /// Update the platform
    pub fn with_platform(mut self, platform: String) -> Self {
        self.platform = Some(platform);
        self
    }
    /// Update the peer ID
    pub fn with_peer_id(mut self, peer_id: String) -> Self {
        self.peer_id = Some(peer_id);
        self
    }
    /// Update the sender ID
    pub fn with_sender_id(mut self, sender_id: String) -> Self {
        self.sender_id = Some(sender_id);
        self
    }
    /// Update the account ID
    pub fn with_account_id(mut self, account_id: String) -> Self {
        self.account_id = Some(account_id);
        self
    }
    /// Update the reasoning level
    pub fn with_reasoning_level(mut self, level: ReasoningLevel) -> Self {
        self.reasoning_level = level;
        self
    }
    /// Update the thread ID
    pub fn with_thread_id(mut self, thread_id: String) -> Self {
        self.thread_id = Some(thread_id);
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningMode {
    /// 直接回答模式
    #[default]
    Direct,
    /// 规划模式（先展示思考框架）
    Plan,
    /// 流式输出模式
    Stream,
    /// 隐藏思考过程模式
    Hidden,
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    /// 活跃状态
    #[default]
    Active,
    /// 已归档状态
    Archived,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Active => write!(f, "active"),
            SessionStatus::Archived => write!(f, "archived"),
        }
    }
}

/// Reasoning Level — 推理深度控制等级
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningLevel {
    /// 低推理深度（最小推理 token 消耗）
    Low,
    /// 中等推理深度
    Medium,
    /// 高推理深度（默认）
    #[default]
    High,
    /// 最大推理深度（最大推理 token 消耗）
    Max,
}

impl std::fmt::Display for ReasoningLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReasoningLevel::Low => write!(f, "low"),
            ReasoningLevel::Medium => write!(f, "medium"),
            ReasoningLevel::High => write!(f, "high"),
            ReasoningLevel::Max => write!(f, "max"),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reasoning_level_default_is_high() {
        assert_eq!(ReasoningLevel::default(), ReasoningLevel::High);
    }

    #[test]
    fn test_reasoning_level_display() {
        assert_eq!(ReasoningLevel::Low.to_string(), "low");
        assert_eq!(ReasoningLevel::Medium.to_string(), "medium");
        assert_eq!(ReasoningLevel::High.to_string(), "high");
        assert_eq!(ReasoningLevel::Max.to_string(), "max");
    }

    #[test]
    fn test_reasoning_level_serde_roundtrip() {
        for level in [
            ReasoningLevel::Low,
            ReasoningLevel::Medium,
            ReasoningLevel::High,
            ReasoningLevel::Max,
        ] {
            let json = serde_json::to_string(&level).unwrap();
            let parsed: ReasoningLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(level, parsed);
        }
    }

    #[test]
    fn test_reasoning_level_deserialize_from_string() {
        assert_eq!(
            serde_json::from_str::<ReasoningLevel>("\"low\"").unwrap(),
            ReasoningLevel::Low
        );
        assert_eq!(
            serde_json::from_str::<ReasoningLevel>("\"medium\"").unwrap(),
            ReasoningLevel::Medium
        );
        assert_eq!(
            serde_json::from_str::<ReasoningLevel>("\"high\"").unwrap(),
            ReasoningLevel::High
        );
        assert_eq!(
            serde_json::from_str::<ReasoningLevel>("\"max\"").unwrap(),
            ReasoningLevel::Max
        );
    }

    #[test]
    fn test_reasoning_level_invalid_value_fails() {
        let result = serde_json::from_str::<ReasoningLevel>("\"extreme\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_session_checkpoint_default_reasoning_level() {
        let checkpoint = SessionCheckpoint::new("sess_1".into());
        assert_eq!(checkpoint.reasoning_level, ReasoningLevel::High);
    }

    #[test]
    fn test_session_checkpoint_with_reasoning_level() {
        let checkpoint =
            SessionCheckpoint::new("sess_2".into()).with_reasoning_level(ReasoningLevel::Low);
        assert_eq!(checkpoint.reasoning_level, ReasoningLevel::Low);
    }
}
