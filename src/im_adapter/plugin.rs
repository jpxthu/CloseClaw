//! Unified IM plugin trait.
//!
//! [`IMPlugin`] unifies inbound parsing, outbound rendering, and outbound
//! sending for each messaging platform. Gateway selects the appropriate plugin
//! by its [`platform`](IMPlugin::platform) identifier.
//!
//! The default [`IMPlugin::render`] implementation provides a generic rendering
//! pipeline that parses content into segments (text, code blocks, horizontal
//! rules) and dispatches to overridable hook methods.  Platform plugins
//! override the hooks they need for platform-specific rendering.

use crate::im_adapter::code_block::{parse_content_segments, ContentSegment};
use crate::im_adapter::error::AdapterError;
use crate::im_adapter::normalized::NormalizedMessage;
use crate::llm::types::ContentBlock;
use crate::processor_chain::DslParseResult;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Output type
// ---------------------------------------------------------------------------

/// Output produced by rendering LLM content for a specific platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedOutput {
    /// Message type, e.g. `"text"` or `"interactive"`.
    pub msg_type: String,
    /// Platform-specific payload JSON.
    pub payload: serde_json::Value,
}

// ---------------------------------------------------------------------------
// IMPlugin trait
// ---------------------------------------------------------------------------

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
/// # Rendering Pipeline
///
/// The default [`render`](IMPlugin::render) implementation:
///
/// 1. Iterates over `content_blocks`.
/// 2. For each [`ContentBlock::Text`], calls
///    [`parse_content_segments`](crate::im_adapter::code_block::parse_content_segments)
///    to split the text into [`ContentSegment`]s.
/// 3. Dispatches each segment to the corresponding hook method
///    ([`render_code_block`], [`render_markdown`], [`render_hr`]).
/// 4. Concatenates hook outputs into a single rendered string.
///
/// Platform plugins override the hook methods to produce platform-native output
/// (e.g. Feishu interactive cards, ANSI-colored terminal text).
///
/// # Example
///
/// A minimal mock implementation using only defaults:
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
    ///
    /// The default implementation parses each [`ContentBlock::Text`] into
    /// segments and dispatches to the corresponding hook method.  Override
    /// this method for a fully custom rendering pipeline.
    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        let mut rendered = String::new();

        for block in content_blocks {
            match block {
                ContentBlock::Text(text) => {
                    let segments = parse_content_segments(text);
                    for segment in segments {
                        match segment {
                            ContentSegment::CodeBlock { language, code } => {
                                rendered.push_str(&self.render_code_block(&language, &code));
                            }
                            ContentSegment::Markdown(text) => {
                                rendered.push_str(&self.render_markdown(&text));
                            }
                            ContentSegment::Hr => {
                                rendered.push_str(&self.render_hr());
                            }
                        }
                    }
                }
                // Non-text blocks are ignored by the default renderer.
                // Platform plugins can override render() to handle them.
                _ => {}
            }
        }

        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::Value::String(rendered),
        }
    }

    /// Render a fenced code block.
    ///
    /// `language` is the optional language annotation from the opening fence
    /// (e.g. `"rust"`, `"python"`).  `code` is the raw code content.
    ///
    /// The default implementation returns a plain-text fenced code block.
    fn render_code_block(&self, language: &str, code: &str) -> String {
        if language.is_empty() {
            format!("```\n{}\n```", code)
        } else {
            format!("```{}\n{}\n```", language, code)
        }
    }

    /// Render a markdown text segment.
    ///
    /// The default implementation returns the text as-is.
    fn render_markdown(&self, text: &str) -> String {
        text.to_string()
    }

    /// Render a horizontal rule.
    ///
    /// The default implementation returns `"---"`.
    fn render_hr(&self) -> String {
        "---".to_string()
    }

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
