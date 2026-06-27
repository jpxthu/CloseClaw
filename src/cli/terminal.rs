//! Terminal channel — adapter and plugin for CLI chat.
//!
//! - [`TerminalAdapter`]: reads user input from stdin, producing
//!   [`NormalizedMessage`]s with blank-line-delimited message boundaries.
//! - [`TerminalPlugin`]: unified IM plugin with ANSI-aware rendering,
//!   delegating outbound rendering to [`TerminalRenderer`].

use crate::im_adapter::normalized::NormalizedMessage;
use crate::im_adapter::plugin::{IMPlugin, RenderedOutput};
use crate::im_adapter::AdapterError;
use async_trait::async_trait;
use closeclaw_common::processor::DslParseResult;
use closeclaw_llm::types::ContentBlock;
use std::io::{self, BufRead, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use super::renderer::TerminalRenderer;

// ---------------------------------------------------------------------------
// TerminalAdapter
// ---------------------------------------------------------------------------

/// Adapter that reads user input from stdin and produces
/// [`NormalizedMessage`]s.
///
/// Messages are delimited by blank lines: a sequence of non-empty lines
/// followed by an empty line forms one message. Trailing newlines within
/// a message are preserved.
#[derive(Debug, Clone, Default)]
pub struct TerminalAdapter;

impl TerminalAdapter {
    /// Create a new adapter.
    pub fn new() -> Self {
        Self
    }

    /// Read a single message from stdin.
    ///
    /// Collects lines until a blank line is encountered, then returns the
    /// joined content as a [`NormalizedMessage`]. Returns `None` when the
    /// input is empty or stdin is exhausted (EOF).
    pub fn read_input(&self) -> Option<NormalizedMessage> {
        let stdin = io::stdin();
        let mut lines = Vec::new();

        for line in stdin.lock().lines() {
            match line {
                Ok(text) => {
                    if text.trim().is_empty() {
                        if lines.is_empty() {
                            continue;
                        }
                        break;
                    }
                    lines.push(text);
                }
                Err(_) => break,
            }
        }

        if lines.is_empty() {
            return None;
        }

        let content = lines.join("\n");
        Some(self.make_message(content))
    }

    /// Build a [`NormalizedMessage`] from raw text content.
    fn make_message(&self, content: String) -> NormalizedMessage {
        NormalizedMessage {
            platform: "terminal".to_string(),
            sender_id: crate::platform::current_uid(),
            peer_id: "cli".to_string(),
            content,
            timestamp: current_timestamp(),
            message_type: "text".to_string(),
            media_refs: vec![],
            quoted_message: None,
            thread_id: None,
            account_id: None,
            card_action: None,
        }
    }
}

/// Return the current Unix timestamp in milliseconds.
fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

// ---------------------------------------------------------------------------
// TerminalPlugin
// ---------------------------------------------------------------------------

/// Unified IM plugin for the terminal (CLI) channel.
///
/// Wraps [`TerminalAdapter`] (stdin input) and [`TerminalRenderer`] (outbound
/// rendering). The default [`render`](IMPlugin::render) pipeline delegates to
/// the renderer.
pub struct TerminalPlugin {
    adapter: TerminalAdapter,
    renderer: TerminalRenderer,
}

impl TerminalPlugin {
    /// Create a new terminal plugin with auto-detected ANSI capability.
    pub fn new() -> Self {
        Self {
            adapter: TerminalAdapter::new(),
            renderer: TerminalRenderer::new(),
        }
    }

    /// Create a terminal plugin with explicit ANSI mode.
    ///
    /// Useful for testing or forcing a specific output mode.
    pub fn with_ansi(ansi: bool) -> Self {
        Self {
            adapter: TerminalAdapter::new(),
            renderer: TerminalRenderer::with_ansi(ansi),
        }
    }
}

impl Default for TerminalPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl IMPlugin for TerminalPlugin {
    fn platform(&self) -> &str {
        "terminal"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        let adapter = self.adapter.clone();
        let result = tokio::task::spawn_blocking(move || adapter.read_input())
            .await
            .map_err(|e| AdapterError::InvalidPayload(e.to_string()))?;
        Ok(result)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        self.renderer.render(content_blocks, dsl_result)
    }

    /// Render a code block with ANSI line numbers and optional syntax highlighting.
    fn render_code_block(&self, language: &str, code: &str) -> String {
        self.renderer.render_code_block(language, code)
    }

    /// Render markdown text with ANSI styling.
    fn render_markdown(&self, text: &str) -> String {
        self.renderer.render_markdown(text)
    }

    /// Render a horizontal rule.
    fn render_hr(&self) -> String {
        self.renderer.render_hr()
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("");

        let mut stdout = io::stdout();
        Write::write_all(&mut stdout, text.as_bytes())
            .map_err(|e| AdapterError::SendFailed(e.to_string()))?;
        Write::flush(&mut stdout).map_err(|e| AdapterError::SendFailed(e.to_string()))?;
        Ok(())
    }

    fn streaming_renderer(
        &self,
    ) -> &std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer> {
        self.renderer.streaming_renderer()
    }
}
