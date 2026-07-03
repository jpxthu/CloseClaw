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

use crate::code_block::{parse_content_segments, ContentSegment};
use crate::error::AdapterError;
use crate::streaming::{DefaultStreamingRenderer, StreamingOutput, StreamingRenderer};
use async_trait::async_trait;
use closeclaw_common::identity::IdentityResolver;
use closeclaw_common::processor::{ContentBlock, DslParseResult, StreamEvent};
use closeclaw_common::NormalizedMessage;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

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
///    [`parse_content_segments`](crate::code_block::parse_content_segments)
///    to split the text into [`ContentSegment`]s.
/// 3. Dispatches each segment to the corresponding hook method
///    ([`render_code_block`], [`render_markdown`], [`render_hr`]).
/// 4. Concatenates hook outputs into a single rendered string.
///
/// Platform plugins override the hook methods to produce platform-native output
/// (e.g. Feishu interactive cards, ANSI-colored terminal text).
#[async_trait]
pub trait IMPlugin: Send + Sync {
    /// Returns the platform identifier, e.g. `"feishu"` or `"discord"`.
    fn platform(&self) -> &str;

    /// Returns the identity resolver for cross-platform account mapping.
    ///
    /// The default implementation returns `None`, meaning no identity
    /// mapping is applied and `account_id` falls back to `sender_id`.
    /// Platforms that support identity mapping override this to return
    /// a resolver instance.
    fn identity_resolver(&self) -> Option<&dyn IdentityResolver> {
        None
    }

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
            if let ContentBlock::Text(text) = block {
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

    /// Clean platform-native text by removing platform-specific markers.
    ///
    /// Receives raw platform text (e.g. Feishu `<at>` tags, Discord mentions)
    /// and returns cleaned plain text. Called by the Processor Chain
    /// ContentNormalizer.
    ///
    /// The default implementation passes text through unchanged.
    fn clean_content(&self, raw: &str) -> String {
        raw.to_string()
    }

    /// Initialize the plugin on startup (connect pool, token, etc.).
    ///
    /// Plugins that do not need initialization can use the default no-op.
    async fn init(&self) -> Result<(), AdapterError> {
        Ok(())
    }

    /// Shut down the plugin and release resources.
    ///
    /// Called during daemon shutdown to clean up connections, caches, etc.
    /// Plugins that do not need cleanup can use the default no-op.
    async fn shutdown(&self) -> Result<(), AdapterError> {
        Ok(())
    }

    /// Shut down the inbound side of the plugin (webhook listeners,
    /// websocket connections for receiving messages).
    ///
    /// Called during Phase 1 of daemon shutdown. The default implementation
    /// delegates to [`shutdown`](IMPlugin::shutdown) for backward compatibility.
    async fn shutdown_inbound(&self) -> Result<(), AdapterError> {
        self.shutdown().await
    }

    /// Shut down the outbound side of the plugin (message sending channels,
    /// API connections for delivering messages).
    ///
    /// Called during Phase 5 of daemon shutdown. The default implementation
    /// delegates to [`shutdown`](IMPlugin::shutdown) for backward compatibility.
    async fn shutdown_outbound(&self) -> Result<(), AdapterError> {
        self.shutdown().await
    }

    /// Process a single streaming [`StreamEvent`] and return incremental
    /// output.
    ///
    /// The default implementation delegates to a [`DefaultStreamingRenderer`]
    /// stored in a [`Mutex`] inside the implementor. Plugins that need
    /// different rendering logic should override this method.
    ///
    /// [`Mutex`] is used because [`IMPlugin`] requires `Send + Sync`.
    fn handle_stream_event(&self, event: StreamEvent) -> StreamingOutput {
        self.streaming_renderer()
            .lock()
            .unwrap()
            .handle_event(event)
    }

    /// Flush the streaming renderer and return any remaining buffered content.
    ///
    /// Called at stream end (e.g. `MessageEnd`) to drain partial lines and
    /// accumulated blocks.
    fn flush_stream(&self) -> StreamingOutput {
        self.streaming_renderer().lock().unwrap().flush()
    }

    /// Access the plugin's streaming renderer.
    ///
    /// The default implementation panics — implementors must override this
    /// to return a [`Mutex<DefaultStreamingRenderer>`] (or a custom renderer
    /// wrapped in one).
    fn streaming_renderer(&self) -> &Mutex<DefaultStreamingRenderer> {
        panic!(
            "IMPlugin::streaming_renderer() not implemented; \
             override handle_stream_event / flush_stream or provide streaming_renderer"
        )
    }
}
