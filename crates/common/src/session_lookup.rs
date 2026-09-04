//! Session lookup trait for decoupling permission from gateway.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
    /// 消息角色（"user" / "assistant"），用于 transcript 格式化
    #[serde(default)]
    pub role: Option<String>,
    /// 目标渠道标识（如 "feishu"、"telegram"），用于 pending_operation 的 target_channel 字段
    #[serde(default)]
    pub target_channel: String,
    /// 出站消息的平台标识（如 "feishu"、"telegram"）
    #[serde(default)]
    pub platform: Option<String>,
    /// DSL 解析结果（序列化 JSON 字符串）
    #[serde(default)]
    pub dsl_result: Option<String>,
    /// 出站消息的内容块（序列化 JSON 字符串，ContentBlock[]）
    #[serde(default)]
    pub content_blocks: Option<String>,
}

impl PendingMessage {
    /// Create a new pending message
    pub fn new(message_id: String, content: String) -> Self {
        Self {
            message_id,
            content,
            created_at: Utc::now(),
            sent: false,
            role: None,
            target_channel: String::new(),
            platform: None,
            dsl_result: None,
            content_blocks: None,
        }
    }

    /// Create a new pending message with an explicit role.
    pub fn with_role(message_id: String, content: String, role: String) -> Self {
        Self {
            message_id,
            content,
            created_at: Utc::now(),
            sent: false,
            role: Some(role),
            target_channel: String::new(),
            platform: None,
            dsl_result: None,
            content_blocks: None,
        }
    }

    /// Create a new pending message with a target channel.
    pub fn with_target_channel(
        message_id: String,
        content: String,
        target_channel: String,
    ) -> Self {
        Self {
            message_id,
            content,
            created_at: Utc::now(),
            sent: false,
            role: None,
            target_channel,
            platform: None,
            dsl_result: None,
            content_blocks: None,
        }
    }

    /// Set the platform identifier.
    pub fn with_platform(mut self, platform: String) -> Self {
        self.platform = Some(platform);
        self
    }

    /// Set the DSL result.
    pub fn with_dsl_result(mut self, dsl_result: String) -> Self {
        self.dsl_result = Some(dsl_result);
        self
    }

    /// Set the content blocks.
    pub fn with_content_blocks(mut self, content_blocks: String) -> Self {
        self.content_blocks = Some(content_blocks);
        self
    }

    /// Mark the message as sent
    pub fn mark_sent(&mut self) {
        self.sent = true;
    }
}

/// Trait for looking up session relationships and pending messages.
///
/// Implemented by `SessionManager` in the gateway crate; used by the
/// permission crate to avoid a direct dependency on gateway.
#[async_trait]
pub trait SessionLookup: Send + Sync {
    /// Get the parent session ID of a given child session.
    async fn get_parent_of(&self, child_id: &str) -> Option<String>;

    /// Get the chat ID associated with a session.
    async fn get_chat_id(&self, session_id: &str) -> Option<String>;

    /// Push a pending message onto a session's queue.
    async fn push_pending_message(
        &self,
        session_id: &str,
        msg: PendingMessage,
    ) -> Result<(), String>;

    /// Get the plan state for a session.
    async fn get_plan_state(&self, session_id: &str) -> Option<crate::PlanState>;

    /// Update the plan state for a session.
    async fn set_plan_state(&self, session_id: &str, plan_state: crate::PlanState);

    /// Switch the session mode (e.g. plan → auto).
    async fn set_session_mode(&self, session_id: &str, mode: crate::SessionMode);

    /// Clear the plan state for a session, destroying any active plan.
    ///
    /// Called when exiting Plan Mode (e.g. `/mode normal`, `/auto`).
    /// Default implementation is a no-op for backward compatibility.
    async fn clear_plan_state(&self, _session_id: &str) {}
}
