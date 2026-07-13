//! Core persistence data structures and service trait
//!
//! Defines the core [`SessionCheckpoint`] structure and [`PersistenceService`] trait
//! for implementing pluggable storage backends.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

use closeclaw_common::communication::CommunicationConfig;
pub use closeclaw_common::{AgentRole, PendingMessage, PlanState, ReasoningLevel, SessionMode};

/// A single ProgressTool call record for recovery fallback.
///
/// Stored in [`SessionCheckpoint::progress_tool_calls`] so that the
/// recovery service can rebuild [`PlanState`] when the first three
/// layers (PlanState persistence, system prompt injection, plan file
/// injection) all fail.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgressToolCallRecord {
    /// Step index (0-based).
    pub step_index: usize,
    /// New status applied by the ProgressTool call.
    pub status: closeclaw_common::ExecutionStepStatus,
    /// Optional summary attached to the call.
    #[serde(default)]
    pub summary: Option<String>,
    /// Optional error message (for failed status).
    #[serde(default)]
    pub error_message: Option<String>,
}

/// A single approval tool call record for recovery layer 3 fallback.
///
/// Stored in [`SessionCheckpoint::approval_tool_calls`] so that the
/// recovery service can inject approval history when the first two
/// layers (PlanState persistence, plan file disk) are unavailable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovalToolCallRecord {
    /// Tool name (e.g. "plan_approval").
    pub tool_name: String,
    /// Plan summary submitted for approval.
    pub plan_summary: String,
    /// Unique request ID for the approval request.
    #[serde(default)]
    pub request_id: Option<String>,
    /// Timestamp of the approval call.
    #[serde(default)]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

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
    /// 父 session ID（spawn 创建时写入，顶层 session 为空）
    #[serde(default)]
    pub parent_session_id: Option<String>,
    /// spawn 层级深度（根节点为 0）
    #[serde(default)]
    pub depth: u32,
    /// 有效最大 spawn 深度预算（沿 spawn 链传播）
    ///
    /// 根 agent 的有效预算 = maxSpawnDepth；
    /// 子 agent 的有效预算 = min(子.maxSpawnDepth, 父.有效预算 - 1)。
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON。
    #[serde(default)]
    pub effective_max_spawn_depth: Option<u32>,
    /// 是否已被 memory-miner 挖掘
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为 false）。
    #[serde(default)]
    pub mined: bool,
    /// memory-miner 挖掘完成的时间戳（Unix 秒）
    ///
    /// `None` 表示尚未挖掘；`Some(ts)` 表示 `mark_mined()` 被调用时的 UTC 时间戳。
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为 None）。
    #[serde(default)]
    pub mined_at: Option<i64>,
    /// dreaming 处理状态（Light → REM → Deep → Completed）
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为 Completed）。
    /// 新建 checkpoint 时默认为 Pending。
    #[serde(default)]
    pub dreaming_status: DreamingStatus,
    /// Pending operations recorded during forceful shutdown.
    ///
    /// Non-empty on restart indicates the session was interrupted mid-operation.
    /// The recovery service uses this to inject failure results and recovery
    /// notifications into the conversation flow.
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为空 Vec）。
    #[serde(default)]
    pub pending_operations: Vec<PendingOperation>,
    /// Recovery notification text to inject into the conversation transcript.
    ///
    /// Built by the recovery service when `pending_operations` is non-empty.
    /// The restore callback reads this field and injects it as a system message
    /// into the session's conversation flow.
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为 None）。
    #[serde(default)]
    pub recovery_notification: Option<String>,
    /// Tool failure results to inject into the conversation transcript.
    ///
    /// For each pending ToolCall operation, a corresponding tool_result message
    /// is built and stored here. The restore callback reads these and injects
    /// them as tool_result entries so the LLM sees natural tool failure responses.
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为空 Vec）。
    #[serde(default)]
    pub pending_tool_failures: Vec<String>,
    /// Verbosity level controlling outbound content filtering.
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为 Full）。
    #[serde(default)]
    pub verbosity_level: closeclaw_common::VerbosityLevel,
    /// Plan Mode 状态（阶段、待办步骤、plan 文件路径）
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为 None）。
    #[serde(default)]
    pub plan_state: Option<PlanState>,
    /// ProgressTool call history for recovery fallback (layer 4).
    ///
    /// Stores serialized ProgressTool calls (step_index, status, summary,
    /// error_message) so that when PlanState checkpoint, system prompt
    /// injection, and plan file injection all fail, the recovery service
    /// can rebuild progress from this history.
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为空 Vec）。
    #[serde(default)]
    pub progress_tool_calls: Vec<ProgressToolCallRecord>,
    /// Approval tool call history for recovery layer 3 fallback.
    ///
    /// Stores approval calls (tool name, plan summary, request ID) so
    /// that when PlanState persistence and plan file disk are unavailable,
    /// the recovery service can inject approval history into `system_appends`.
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为空 Vec）。
    #[serde(default)]
    pub approval_tool_calls: Vec<ApprovalToolCallRecord>,
    /// Plan-related references extracted from session message history.
    ///
    /// Stores plan-related text snippets (e.g. plan summaries, file paths)
    /// extracted from user messages. Used by recovery layer 4 fallback to
    /// inject context when the first three layers are all unavailable.
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为空 Vec）。
    #[serde(default)]
    pub plan_references: Vec<String>,
    /// Session Mode — controls session-level behavior constraints.
    ///
    /// `SessionMode` is **orthogonal** to `ReasoningMode` / `ReasoningModeState`:
    /// - `ReasoningMode` governs LLM reasoning presentation (Direct/Plan/Stream/Hidden)
    /// - `SessionMode` governs session behavior constraints (Normal/Plan/Auto)
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为 Normal）。
    #[serde(default)]
    pub session_mode: SessionMode,
    /// 压缩后的对话 transcript（仅 boundary 消息，不含 system prompt）
    ///
    /// 设计文档要求"transcript 是唯一真实来源"，压缩后完整写入持久化存储。
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON。
    #[serde(default)]
    pub transcript: Vec<crate::llm_session::SessionMessage>,
    /// 子 session 简短标签（spawn 时传入，用于 UI 展示和调试）
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为 None）。
    #[serde(default)]
    pub label: Option<String>,
    /// Communication configuration for spawned child sessions.
    ///
    /// Stores the outbound/inbound whitelist that controls which agents
    /// the child session may communicate with. Persisted to checkpoint
    /// so routing restrictions survive gateway restart.
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为 None）。
    #[serde(default)]
    pub communication_config: Option<CommunicationConfig>,
    /// Spawn mode — records whether this child session was created with
    /// `"run"` or `"session"` mode. Used by `rebuild_spawn_tree()` to
    /// restore the original mode after gateway restart.
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为 None）。
    #[serde(default)]
    pub spawn_mode: Option<String>,
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
            parent_session_id: None,
            depth: 0,
            effective_max_spawn_depth: None,
            mined: false,
            mined_at: None,
            dreaming_status: DreamingStatus::Pending,
            pending_operations: Vec::new(),
            recovery_notification: None,
            pending_tool_failures: Vec::new(),
            verbosity_level: closeclaw_common::VerbosityLevel::default(),
            plan_state: None,
            progress_tool_calls: Vec::new(),
            approval_tool_calls: Vec::new(),
            plan_references: Vec::new(),
            session_mode: SessionMode::default(),
            transcript: Vec::new(),
            label: None,
            communication_config: None,
            spawn_mode: None,
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
    /// Update the parent session ID
    pub fn with_parent_session_id(mut self, parent: String) -> Self {
        self.parent_session_id = Some(parent);
        self
    }
    /// Update the spawn depth
    pub fn with_depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }
    /// Update the effective max spawn depth
    pub fn with_effective_max_spawn_depth(mut self, depth: Option<u32>) -> Self {
        self.effective_max_spawn_depth = depth;
        self
    }
    /// Update the mined flag
    pub fn with_mined(mut self, mined: bool) -> Self {
        self.mined = mined;
        if mined && self.mined_at.is_none() {
            self.mined_at = Some(Utc::now().timestamp());
        }
        self
    }
    /// Update the dreaming status
    pub fn with_dreaming_status(mut self, status: DreamingStatus) -> Self {
        self.dreaming_status = status;
        self
    }
    /// Update the pending operations list
    pub fn with_pending_operations(mut self, ops: Vec<PendingOperation>) -> Self {
        self.pending_operations = ops;
        self
    }
    /// Set the recovery notification text
    pub fn with_recovery_notification(mut self, text: Option<String>) -> Self {
        self.recovery_notification = text;
        self
    }
    /// Set the pending tool failure results
    pub fn with_pending_tool_failures(mut self, failures: Vec<String>) -> Self {
        self.pending_tool_failures = failures;
        self
    }
    /// Update the verbosity level
    pub fn with_verbosity_level(mut self, level: closeclaw_common::VerbosityLevel) -> Self {
        self.verbosity_level = level;
        self
    }
    /// Update the plan state
    pub fn with_plan_state(mut self, state: PlanState) -> Self {
        self.plan_state = Some(state);
        self
    }
    /// Update the session mode
    pub fn with_session_mode(mut self, mode: SessionMode) -> Self {
        self.session_mode = mode;
        self
    }
    /// Set the ProgressTool call history for recovery fallback (layer 4).
    pub fn with_progress_tool_calls(mut self, records: Vec<ProgressToolCallRecord>) -> Self {
        self.progress_tool_calls = records;
        self
    }
    /// Record a ProgressTool call in the checkpoint's history.
    ///
    /// Appends the call record so it is available during recovery
    /// as a layer 4 fallback when the first three layers fail.
    pub fn record_progress_call(&mut self, record: ProgressToolCallRecord) {
        self.progress_tool_calls.push(record);
    }
    /// Set the approval tool call history for recovery layer 3 fallback.
    pub fn with_approval_tool_calls(mut self, records: Vec<ApprovalToolCallRecord>) -> Self {
        self.approval_tool_calls = records;
        self
    }
    /// Record an approval tool call in the checkpoint's history.
    pub fn record_approval_call(&mut self, record: ApprovalToolCallRecord) {
        self.approval_tool_calls.push(record);
    }
    /// Set the plan references for recovery layer 4 fallback.
    pub fn with_plan_references(mut self, refs: Vec<String>) -> Self {
        self.plan_references = refs;
        self
    }
    /// Add a plan reference extracted from session message history.
    pub fn add_plan_reference(&mut self, reference: String) {
        self.plan_references.push(reference);
    }
    /// Set the short label for the child session.
    pub fn with_label(mut self, label: String) -> Self {
        self.label = Some(label);
        self
    }
    /// Set the communication configuration.
    pub fn with_communication_config(mut self, config: CommunicationConfig) -> Self {
        self.communication_config = Some(config);
        self
    }
    /// Set the spawn mode ("run" or "session").
    pub fn with_spawn_mode(mut self, mode: String) -> Self {
        self.spawn_mode = Some(mode);
        self
    }
    /// Touch the updated_at timestamp
    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }
}

/// Reasoning Mode State — 推理模式的状态
///
/// **Orthogonal to `SessionMode`**: `ReasoningModeState` tracks LLM
/// reasoning step progress (current_step, is_complete), while
/// `SessionMode` controls session-level behavior constraints
/// (tool visibility, permission boundaries, prompt instructions).
/// The two are stored independently and switched independently.
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

/// Dreaming Status — 会话 dreaming 处理状态
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DreamingStatus {
    /// 未开始 dreaming（初始状态）
    #[serde(rename = "pending")]
    Pending,
    /// Light 阶段处理中
    #[serde(rename = "in_light")]
    InLight,
    /// REM 阶段处理中
    #[serde(rename = "in_rem")]
    InRem,
    /// Deep 阶段处理中
    #[serde(rename = "in_deep")]
    InDeep,
    /// dreaming 完成
    #[default]
    #[serde(rename = "completed")]
    Completed,
}

impl std::fmt::Display for DreamingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DreamingStatus::Pending => write!(f, "pending"),
            DreamingStatus::InLight => write!(f, "in_light"),
            DreamingStatus::InRem => write!(f, "in_rem"),
            DreamingStatus::InDeep => write!(f, "in_deep"),
            DreamingStatus::Completed => write!(f, "completed"),
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

/// Convert DreamingStatus to/from database string representation
pub fn dreaming_status_to_db(s: &DreamingStatus) -> &'static str {
    match s {
        DreamingStatus::Pending => "pending",
        DreamingStatus::InLight => "in_light",
        DreamingStatus::InRem => "in_rem",
        DreamingStatus::InDeep => "in_deep",
        DreamingStatus::Completed => "completed",
    }
}

/// Convert database string to DreamingStatus
pub fn dreaming_status_from_db(s: &str) -> DreamingStatus {
    match s {
        "pending" => DreamingStatus::Pending,
        "in_light" => DreamingStatus::InLight,
        "in_rem" => DreamingStatus::InRem,
        "in_deep" => DreamingStatus::InDeep,
        unknown => {
            warn!(unknown_status = %unknown, "unknown dreaming_status from DB, defaulting to Pending");
            DreamingStatus::Pending
        }
    }
}

/// Type of pending operation recorded in a checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingOperationType {
    /// A tool call that was in progress when the session stopped.
    ToolCall,
    /// A sub-session spawn that was initiated but not confirmed complete.
    SubSessionSpawn,
    /// An outbound message that was queued but not confirmed delivered.
    OutboundMessage,
}

/// A pending operation recorded in a checkpoint during forceful shutdown.
///
/// When the daemon restarts, these entries allow the recovery service to
/// inject failure results into the conversation flow so the LLM can
/// decide whether to retry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingOperation {
    /// Unique identifier for this operation (e.g. tool call id, child session id).
    pub op_id: String,
    /// Type of the pending operation.
    pub op_type: PendingOperationType,
    /// Human-readable name (tool name, session id, channel name).
    pub name: String,
    /// Serialized arguments (tool args JSON, session config, message content).
    #[serde(default)]
    pub args: String,
    /// When this operation was initiated.
    pub created_at: DateTime<Utc>,
}

/// Persistence errors
/// Result of a data consistency check between SQLite and the file system.
#[derive(Debug, Default, Clone)]
pub struct ConsistencyCheckResult {
    /// Number of SQLite records deleted because their transcript files were missing.
    pub deleted_orphaned_records: u64,
    /// Number of orphan transcript files deleted because they had no SQLite record.
    pub deleted_orphaned_files: u64,
}

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
}
