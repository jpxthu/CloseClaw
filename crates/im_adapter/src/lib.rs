//! IM Adapter — Core types and rendering abstractions for IM adapters
//!
//! This crate unifies IMPlugin, IMAdapter, NormalizedMessage, AdapterError,
//! and RenderedOutput under a single entry point.

#[cfg(test)]
mod dependency_boundary_tests;
pub mod error;
pub mod lazy_tool;
#[cfg(test)]
pub mod lazy_tool_tests;
pub mod media_store;
pub mod normalized;
#[cfg(test)]
pub mod normalized_tests;
pub mod platforms;
#[cfg(test)]
pub mod plugin_tests;
pub mod ports;
#[cfg(test)]
pub mod streaming_tests;
pub mod tool_registrar;

pub use error::AdapterError;
pub use tool_registrar::ImAdapterToolsRegistrar;

use async_trait::async_trait;

/// IM Adapter trait - implemented by each messaging platform.
#[async_trait]
pub trait IMAdapter: Send + Sync {
    /// Platform name (e.g., "feishu", "discord")
    fn name(&self) -> &str;

    /// Parse an inbound webhook payload into a [`NormalizedMessage`].
    ///
    /// Returns `Ok(Some(msg))` for recognized message events,
    /// `Ok(None)` for non-message events (e.g. card actions) or
    /// payloads that should be silently ignored, or `Err` on parse failure.
    async fn parse_inbound(
        &self,
        payload: &[u8],
    ) -> Result<Option<closeclaw_common::NormalizedMessage>, AdapterError>;

    /// Parse an inbound webhook payload into a [`CardActionEvent`].
    ///
    /// Returns `Ok(Some(event))` for card-action events,
    /// `Ok(None)` for non-card-action events, or `Err` on parse failure.
    async fn parse_card_action(
        &self,
        payload: &[u8],
    ) -> Result<Option<closeclaw_common::CardActionEvent>, AdapterError>;

    /// Send message to IM platform.
    ///
    /// `to` is the target chat, `content` the plain-text body, and
    /// `root_id` optionally directs the message into a specific
    /// thread/topic (e.g. Feishu `root_id` query parameter).
    async fn send_message(
        &self,
        to: &str,
        content: &str,
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
