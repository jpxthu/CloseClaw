//! im_adapter — Core types and rendering abstractions for IM adapters
//!
//! This module unifies IMPlugin, IMAdapter, NormalizedMessage, AdapterError,
//! and RenderedOutput under a single entry point.

pub mod code_block;
pub mod error;
pub mod normalized;
pub mod normalized_tests;
pub mod platforms;
pub mod plugin;
pub mod plugin_tests;
pub mod streaming;

pub use error::AdapterError;
pub use normalized::NormalizedMessage;
pub use plugin::{IMPlugin, RenderedOutput};

use crate::gateway::Message;
use async_trait::async_trait;

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
    async fn handle_webhook(&self, payload: &[u8]) -> Result<Option<Message>, AdapterError>;

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

    /// Close inbound connections (e.g. unsubscribe webhook, disconnect WebSocket).
    ///
    /// Called during daemon Phase 1 (inbound shutdown) to stop accepting new
    /// messages from the platform. Default implementation is a no-op.
    async fn close_inbound(&self) -> Result<(), AdapterError> {
        Ok(())
    }

    /// Close outbound connections (e.g. drain send queue, disconnect API client).
    ///
    /// Called during daemon Phase 5 (outbound shutdown) to stop sending
    /// messages to the platform. Default implementation is a no-op.
    async fn close_outbound(&self) -> Result<(), AdapterError> {
        Ok(())
    }
}
