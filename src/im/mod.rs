//! IM Adapters - Protocol implementations for various messaging platforms
//!
//! Each adapter implements the IMAdapter trait for a specific IM platform.

pub mod feishu;
#[cfg(test)]
pub mod feishu_tests;
pub mod normalized;
#[cfg(test)]
pub mod normalized_tests;
pub mod plugin;
pub mod processor;
pub use normalized::NormalizedMessage;
pub use plugin::IMPlugin;

use crate::gateway::Message;
use async_trait::async_trait;

/// IM Adapter trait - implemented by each messaging platform
#[async_trait]
pub trait IMAdapter: Send + Sync {
    /// Platform name (e.g., "feishu", "discord")
    fn name(&self) -> &str;

    /// Handle incoming message from IM platform
    async fn handle_webhook(&self, payload: &[u8]) -> Result<Message, AdapterError>;

    /// Send message to IM platform.
    ///
    /// `root_id` optionally directs the message into a specific thread/topic
    /// (e.g. Feishu `root_id` query parameter).
    async fn send_message(
        &self,
        message: &Message,
        root_id: Option<&str>,
    ) -> Result<(), AdapterError>;

    /// Validate webhook signature
    async fn validate_signature(&self, signature: &str, payload: &[u8]) -> bool;

    /// Send an interactive card message using pre-serialized JSON.
    ///
    /// `root_id` optionally directs the message into a specific thread/topic.
    /// Returns `AdapterError::UnsupportedOperation` by default.
    async fn send_card_json(
        &self,
        _chat_id: &str,
        _card_json: &str,
        _root_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        Err(AdapterError::UnsupportedOperation)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("Invalid payload: {0}")]
    InvalidPayload(String),

    #[error("Authentication failed")]
    AuthFailed,

    #[error("Send failed: {0}")]
    SendFailed(String),

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Unsupported operation")]
    UnsupportedOperation,
}
