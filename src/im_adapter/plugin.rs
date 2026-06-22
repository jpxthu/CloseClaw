//! Unified IM plugin trait.
//!
//! [`IMPlugin`] unifies inbound parsing, outbound rendering, and outbound
//! sending for each messaging platform. Gateway selects the appropriate plugin
//! by its [`platform`](IMPlugin::platform) identifier.

use crate::im_adapter::AdapterError;
use crate::im_adapter::NormalizedMessage;
use crate::llm::types::ContentBlock;
use crate::processor_chain::DslParseResult;
use crate::renderer::RenderedOutput;
use async_trait::async_trait;

/// Unified plugin trait for messaging platforms.
///
/// Each platform (Feishu, Discord, etc.) implements this trait to provide:
///
/// - **Inbound**: Parse a raw webhook payload into a [`NormalizedMessage`].
/// - **Outbound rendering**: Convert LLM [`ContentBlock`]s into platform-native
///   [`RenderedOutput`].
/// - **Outbound sending**: Deliver the rendered output to the platform.
///
/// Gateway maintains a `platform → IMPlugin` registry and routes messages
/// through the matching plugin without caring about platform internals.
///
/// # Example
///
/// A minimal mock implementation:
///
/// ```rust
/// use async_trait::async_trait;
/// use closeclaw::im::{IMPlugin, AdapterError, NormalizedMessage};
/// use closeclaw::llm::types::ContentBlock;
/// use closeclaw::processor_chain::DslParseResult;
/// use closeclaw::renderer::RenderedOutput;
///
/// struct MockPlugin;
///
/// #[async_trait]
/// impl IMPlugin for MockPlugin {
///     fn platform(&self) -> &str {
///         "mock"
///     }
///
///     async fn parse_inbound(
///         &self,
///         _payload: &[u8],
///     ) -> Result<Option<NormalizedMessage>, AdapterError> {
///         Ok(None)
///     }
///
///     fn render(
///         &self,
///         _content_blocks: &[ContentBlock],
///         _dsl_result: Option<&DslParseResult>,
///     ) -> RenderedOutput {
///         RenderedOutput {
///             msg_type: "text".into(),
///             payload: serde_json::Value::Null,
///         }
///     }
///
///     async fn send(
///         &self,
///         _output: &RenderedOutput,
///         _peer_id: &str,
///         _thread_id: Option<&str>,
///     ) -> Result<(), AdapterError> {
///         Ok(())
///     }
/// }
///
/// let plugin = MockPlugin;
/// assert_eq!(plugin.platform(), "mock");
/// ```
#[async_trait]
pub trait IMPlugin: Send + Sync {
    /// Returns the platform identifier, e.g. `"feishu"` or `"discord"`.
    fn platform(&self) -> &str;

    /// Parse an inbound webhook payload into a [`NormalizedMessage`].
    ///
    /// Returns `Ok(None)` when the payload should be silently ignored (e.g.
    /// empty content, unsupported message type). Returns `Err` on parse failure.
    async fn parse_inbound(
        &self,
        payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError>;

    /// Validate the webhook signature.
    ///
    /// The default implementation always returns `true`. Platforms that require
    /// signature verification should override this.
    async fn validate_signature(&self, _signature: &str, _payload: &[u8]) -> bool {
        true
    }

    /// Render LLM content blocks into a platform-native output.
    ///
    /// `dsl_result` carries parsed DSL instructions from the processor chain
    /// and may be `None` when no DSL was extracted.
    fn render(
        &self,
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput;

    /// Send the rendered output to the platform.
    ///
    /// `peer_id` identifies the target chat or user. `thread_id` optionally
    /// directs the message into a specific thread/topic.
    async fn send(
        &self,
        output: &RenderedOutput,
        peer_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), AdapterError>;

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
