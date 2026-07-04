//! Rendering hook trait for IM platform plugins.
//!
//! [`RenderHooks`] provides platform-specific text rendering hooks
//! (code blocks, markdown, horizontal rules) used by the default
//! [`IMPlugin::render`](crate::plugin::IMPlugin::render) pipeline.

/// Rendering hooks for platform-specific text formatting.
///
/// Platform plugins implement these methods to customize how fenced code
/// blocks, markdown segments, and horizontal rules are rendered.  The
/// default [`IMPlugin::render`](crate::plugin::IMPlugin::render) pipeline
/// calls these hooks for each content segment.
pub trait RenderHooks: Send + Sync {
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
}
