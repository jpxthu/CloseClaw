//! Structured detail for pending operations.
//!
//! Defines [`PendingOperationDetail`] with type-specific fields for each
//! operation type, as specified by the session-lifecycle design document.

use serde::{Deserialize, Serialize};

/// Structured supplementary information for a pending operation,
/// varying by `PendingOperationType`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "variant", rename_all = "snake_case")]
pub enum PendingOperationDetail {
    /// Tool call in progress.
    ToolCall {
        /// Tool name (e.g. "bash", "web_search").
        tool_name: String,
        /// Serialized arguments or parameter summary.
        args_summary: String,
    },
    /// Sub-session spawn in progress.
    SubSessionSpawn {
        /// Child session identifier.
        child_session_id: String,
        /// Agent identifier.
        agent_id: String,
        /// Task summary / description.
        task_summary: String,
    },
    /// Outbound message pending delivery.
    OutboundMessage {
        /// Target channel name or identifier.
        target_channel: String,
        /// Platform-specific message identifier.
        message_id: String,
        /// Delivery status (e.g. "pending", "sent").
        delivery_status: String,
    },
}

impl PendingOperationDetail {
    /// Extract the tool name if this is a ToolCall variant.
    pub fn tool_name(&self) -> Option<&str> {
        match self {
            PendingOperationDetail::ToolCall { tool_name, .. } => Some(tool_name),
            _ => None,
        }
    }

    /// Extract the tool args summary if this is a ToolCall variant.
    pub fn args_summary(&self) -> Option<&str> {
        match self {
            PendingOperationDetail::ToolCall { args_summary, .. } => Some(args_summary),
            _ => None,
        }
    }

    /// Extract the child session id if this is a SubSessionSpawn variant.
    pub fn child_session_id(&self) -> Option<&str> {
        match self {
            PendingOperationDetail::SubSessionSpawn {
                child_session_id, ..
            } => Some(child_session_id),
            _ => None,
        }
    }

    /// Extract the target channel if this is an OutboundMessage variant.
    pub fn target_channel(&self) -> Option<&str> {
        match self {
            PendingOperationDetail::OutboundMessage { target_channel, .. } => Some(target_channel),
            _ => None,
        }
    }

    /// Extract the message id if this is an OutboundMessage variant.
    pub fn message_id(&self) -> Option<&str> {
        match self {
            PendingOperationDetail::OutboundMessage { message_id, .. } => Some(message_id),
            _ => None,
        }
    }

    /// Extract the delivery status if this is an OutboundMessage variant.
    pub fn delivery_status(&self) -> Option<&str> {
        match self {
            PendingOperationDetail::OutboundMessage {
                delivery_status, ..
            } => Some(delivery_status),
            _ => None,
        }
    }
}
