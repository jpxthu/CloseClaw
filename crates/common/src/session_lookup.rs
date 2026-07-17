//! Session lookup trait for decoupling permission from gateway.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ModeTransition;

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
        }
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

    /// Set a pending mode transition to be injected into the next system prompt.
    async fn set_pending_mode_transition(&self, session_id: &str, transition: ModeTransition);
}
