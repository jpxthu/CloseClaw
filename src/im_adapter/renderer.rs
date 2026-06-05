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
