//! IM Adapters - Protocol implementations for various messaging platforms
//!
//! Each adapter implements the IMAdapter trait for a specific IM platform.

pub mod feishu;

use async_trait::async_trait;
use crate::gateway::Message;

/// IM Adapter trait - implemented by each messaging platform
#[async_trait]
pub trait IMAdapter: Send + Sync {
    /// Platform name (e.g., "feishu", "discord")
    fn name(&self) -> &str;

    /// Handle incoming message from IM platform
    async fn handle_webhook(&self, payload: &[u8]) -> Result<Message, AdapterError>;

    /// Send message to IM platform
    async fn send_message(&self, message: &Message) -> Result<(), AdapterError>;

    /// Validate webhook signature
    async fn validate_signature(&self, signature: &str, payload: &[u8]) -> bool;
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
}
