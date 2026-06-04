//! Renderer — rendering layer infrastructure.
//!
//! This module provides the core types and trait for rendering LLM output
//! to platform-specific formats:
//! - [`RenderedOutput`] — output type from a renderer
//! - [`Renderer`] — trait for platform-specific renderers
//!
//! # Example
//! ```
//! use closeclaw::renderer::{Renderer, RenderedOutput};
//! use closeclaw::renderer::feishu::FeishuRenderer;
//! use closeclaw::llm::types::ContentBlock;
//!
//! let renderer = FeishuRenderer::new();
//! // A single Text block containing inline formatting (**) → interactive card.
//! // With dsl_result=None there are no DSL buttons in the payload.
//! let blocks = vec![ContentBlock::Text("Hello **world**".to_string())];
//! let output = renderer.render(&blocks, None);
//! assert_eq!(output.msg_type, "interactive");
//! // No DSL → no action/button elements in the card
//! let elements = output.payload.get("card").and_then(|c| c.get("elements")).and_then(|e| e.as_array());
//! assert!(elements.is_some());
//! assert!(elements.unwrap().iter().all(|e| e.get("tag").and_then(|t| t.as_str()) != Some("action")));
//! ```

pub mod code_block;
pub mod feishu;
pub mod streaming;

use serde::{Deserialize, Serialize};

use crate::llm::types::ContentBlock;
use crate::processor_chain::DslParseResult;

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

/// Output produced by a [`Renderer`] after rendering LLM content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedOutput {
    /// Message type, e.g. `"text"` or `"interactive"`.
    pub msg_type: String,
    /// Platform-specific payload JSON.
    pub payload: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Renderer trait
// ---------------------------------------------------------------------------

/// Trait for rendering LLM output to a platform-specific format.
///
/// Implementors must be `Send + Sync` to allow sharing across async contexts.
pub trait Renderer: Send + Sync {
    /// Returns the platform name, e.g. `"feishu"` or `"wecom"`.
    fn platform(&self) -> &str;

    /// Renders structured LLM output (`content_blocks`) to a platform-specific
    /// [`RenderedOutput`].
    ///
    /// `dsl_result` carries parsed DSL instructions from the processor chain
    /// and may be `None` when no DSL was extracted. Renderers are expected to
    /// dispatch by [`ContentBlock`] variant: Text/Thinking/ToolUse/ToolResult
    /// each get their own rendering path. A single Text block with simple
    /// content should produce a `"text"` output for backward compatibility.
    fn render(
        &self,
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput;
}
