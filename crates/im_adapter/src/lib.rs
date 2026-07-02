//! IM Adapter — Core types and rendering abstractions for IM adapters
//!
//! This crate unifies IMPlugin, IMAdapter, NormalizedMessage, AdapterError,
//! and RenderedOutput under a single entry point.

pub mod code_block;
pub mod error;
pub mod normalized;
#[cfg(test)]
pub mod normalized_tests;
pub mod platforms;
pub mod plugin;
#[cfg(test)]
pub mod plugin_tests;
pub mod streaming;
#[cfg(test)]
pub mod streaming_tests;
pub mod tool_registrar;

pub use error::AdapterError;
pub use normalized::NormalizedMessage;
pub use plugin::{IMPlugin, RenderedOutput};
pub use tool_registrar::ImAdapterToolsRegistrar;

use async_trait::async_trait;
use closeclaw_gateway::Message;

/// IM Adapter trait - implemented by each messaging platform.
#[async_trait]
pub trait IMAdapter: Send + Sync {
    /// Platform name (e.g., "feishu", "discord")
    fn name(&self) -> &str;

    /// Handle incoming event from IM platform.
    ///
    /// Returns `Ok(Some(message))` for recognized message events,
    /// `Ok(None)` for events that should be silently ignored
    /// (e.g. unknown card actions), or `Err` on parse failure.
    async fn handle_webhook(
        &self,
        payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError>;

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
