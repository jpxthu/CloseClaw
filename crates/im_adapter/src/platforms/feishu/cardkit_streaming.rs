//! Feishu cardkit streaming renderer.
//!
//! Implements the three-step cardkit protocol for streaming LLM output:
//! 1. Create a streaming card (`POST /cardkit/v1/cards`, `streaming_mode: true`)
//! 2. Send a card-reference message (card_json with `card_id`)
//! 3. Batch-update card element content (`PUT /cardkit/v1/cards/{card_id}/elements/{element_id}/content`)
//!
//! Line buffering rules are inherited from `docs/design/im_adapter/streaming-render.md`.

use std::time::{Duration, Instant};

use super::adapter::FeishuAdapter;
use super::send_helpers::run_cli;
use closeclaw_common::streaming::LineBuffer;

#[allow(dead_code)]
const LINE_THRESHOLD: usize = 100;

/// Default timeout for forced buffer emission.
#[allow(dead_code)]
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(200);

/// Cardkit streaming element ID for the main content element.
pub(crate) const STREAMING_ELEMENT_ID: &str = "streaming_content";

/// Streaming card state managed per-response.
pub(crate) struct FeishuStreamingState {
    /// Card ID assigned by cardkit on creation.
    pub(crate) card_id: Option<String>,
    /// Monotonically increasing sequence for card updates.
    pub(crate) sequence: u32,
    /// Line buffer for text content.
    pub(crate) line_buffer: LineBuffer,
    /// Whether streaming is active (card created, updates in progress).
    pub(crate) is_active: bool,
    /// Timestamp of the last card update sent to the platform.
    pub(crate) last_update: Option<Instant>,
    /// Accumulated text since the last card update.
    pub(crate) pending_text: String,
}

impl Default for FeishuStreamingState {
    fn default() -> Self {
        Self {
            card_id: None,
            sequence: 0,
            line_buffer: LineBuffer::new(),
            is_active: false,
            last_update: None,
            pending_text: String::new(),
        }
    }
}

impl FeishuStreamingState {
    /// Reset all streaming state for a new response.
    #[allow(dead_code)]
    pub(crate) fn reset(&mut self) {
        self.card_id = None;
        self.sequence = 0;
        self.line_buffer.reset();
        self.is_active = false;
        self.last_update = None;
        self.pending_text.clear();
    }
}

/// Feishu cardkit streaming renderer.
///
/// Manages the cardkit three-step protocol (create → send ref → batch update)
/// and enforces line buffering rules from `streaming-render.md`.
pub(crate) struct CardkitStreamingRenderer {
    pub(crate) state: FeishuStreamingState,
}

impl CardkitStreamingRenderer {
    /// Create a new cardkit streaming renderer with WholeBlock code mode.
    pub(crate) fn new() -> Self {
        Self {
            state: FeishuStreamingState::default(),
        }
    }

    /// Handle a text delta from the LLM stream.
    ///
    /// Feeds text through the line buffer and returns any completed lines.
    /// Lines are buffered internally for batch card updates.
    pub(crate) fn handle_text_delta(&mut self, text: &str) {
        for line in self.state.line_buffer.feed(text) {
            self.state.pending_text.push_str(&line);
        }
    }

    /// Handle a BlockStart event for a Text block.
    ///
    /// Resets the line buffer for the new text block.
    pub(crate) fn handle_block_start_text(&mut self) {
        self.state.line_buffer.reset();
    }

    /// Handle a BlockEnd event for a Text block.
    ///
    /// Flushes any remaining buffered text.
    pub(crate) fn handle_block_end_text(&mut self) {
        if let Some(remaining) = self.state.line_buffer.flush() {
            self.state.pending_text.push_str(&remaining);
        }
    }

    /// Check the line buffer timeout; if elapsed, force-output buffered content.
    ///
    /// Returns the pending text if the timeout has been exceeded.
    pub(crate) fn check_timeout(&mut self) -> Option<String> {
        if self.state.pending_text.is_empty() {
            return None;
        }
        if let Some(lines) = self.state.line_buffer.check_timeout() {
            for line in lines {
                self.state.pending_text.push_str(&line);
            }
        }
        if self.state.pending_text.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.state.pending_text))
        }
    }

    /// Flush all remaining buffered content.
    ///
    /// Called at MessageEnd to drain the line buffer and return all
    /// pending text.
    pub(crate) fn flush(&mut self) -> String {
        if let Some(remaining) = self.state.line_buffer.flush() {
            self.state.pending_text.push_str(&remaining);
        }
        std::mem::take(&mut self.state.pending_text)
    }

    /// Check if a card update should be sent based on timing.
    ///
    /// Enforces a minimum interval between card updates to avoid
    /// exceeding the platform's update frequency limit.
    #[allow(dead_code)]
    pub(crate) fn should_update_now(&self) -> bool {
        match self.state.last_update {
            Some(last) => last.elapsed() >= Duration::from_millis(100),
            None => true,
        }
    }

    /// Create a streaming card via cardkit API.
    ///
    /// Returns `Some(card_id)` on success, `None` on failure.
    #[allow(dead_code)]
    pub(crate) async fn create_card(adapter: &FeishuAdapter) -> Option<String> {
        let card_json = serde_json::json!({
            "type": "card_json",
            "card": {
                "streaming_mode": true,
                "body": {
                    "elements": [{
                        "tag": "markdown",
                        "element_id": STREAMING_ELEMENT_ID,
                        "content": ""
                    }]
                }
            }
        });

        let params = serde_json::json!({
            "card": card_json
        });
        let params_str = params.to_string();

        let args = [
            "api",
            "--method",
            "POST",
            "--uri",
            "/open-apis/cardkit/v1/cards",
            "--params",
            &params_str,
        ];

        match run_cli(adapter, &args).await {
            Ok(stdout) => {
                let val: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_default();
                val.get("data")
                    .and_then(|d| d.get("card_id"))
                    .and_then(|id| id.as_str())
                    .map(String::from)
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to create streaming card");
                None
            }
        }
    }

    /// Send a card-reference message via lark-cli.
    ///
    /// Sends an interactive card message containing a reference to the
    /// streaming card by card_id.
    #[allow(dead_code)]
    pub(crate) async fn send_card_ref(
        adapter: &FeishuAdapter,
        chat_id: &str,
        card_id: &str,
        root_id: Option<&str>,
    ) -> Result<(), crate::error::AdapterError> {
        let card_json = serde_json::json!({
            "type": "card_json",
            "card_id": card_id
        });
        let content = card_json.to_string();
        adapter
            .send_msg(chat_id, "interactive", &content, root_id)
            .await
    }

    /// Update a card element's content via cardkit API.
    ///
    /// Sends a PUT request to update the markdown content of a specific
    /// element within a streaming card. The sequence number must be
    /// monotonically increasing for idempotent updates.
    pub(crate) async fn update_element(
        adapter: &FeishuAdapter,
        card_id: &str,
        element_id: &str,
        content: &str,
        sequence: u32,
    ) -> Result<(), crate::error::AdapterError> {
        let update_json = serde_json::json!({
            "content": content
        });
        let update_str = update_json.to_string();

        let uri = format!(
            "/open-apis/cardkit/v1/cards/{}/elements/{}/content",
            card_id, element_id
        );
        let header = format!("X-Cardkit-Sequence: {}", sequence);

        let args = [
            "api",
            "--method",
            "PUT",
            "--uri",
            &uri,
            "--params",
            &update_str,
            "--header",
            &header,
        ];

        run_cli(adapter, &args).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_renderer() -> CardkitStreamingRenderer {
        CardkitStreamingRenderer::new()
    }

    #[test]
    fn text_delta_buffers_content() {
        let mut r = make_renderer();
        r.handle_text_delta("Hello ");
        assert!(r.state.pending_text.is_empty());
        r.handle_text_delta("world!");
        // The `!` is a sentence terminator, so LineBuffer emits
        // "Hello world!" into pending_text
        assert_eq!(r.state.pending_text, "Hello world!");
    }

    #[test]
    fn text_delta_with_sentence_terminator_emits() {
        let mut r = make_renderer();
        r.handle_text_delta("Hello world! ");
        // `!` triggers emit of "Hello world!", trailing space stays in buffer
        assert_eq!(r.state.pending_text, "Hello world!");
    }

    #[test]
    fn text_delta_with_newline_emits() {
        let mut r = make_renderer();
        r.handle_text_delta("line1\nline2");
        assert_eq!(r.state.pending_text, "line1\n");
    }

    #[test]
    fn block_start_resets_line_buffer() {
        let mut r = make_renderer();
        r.handle_text_delta("partial");
        r.handle_block_start_text();
        assert!(r.state.line_buffer.flush().is_none());
    }

    #[test]
    fn block_end_flushes_remaining() {
        let mut r = make_renderer();
        r.handle_text_delta("no terminator here");
        r.handle_block_end_text();
        assert_eq!(r.state.pending_text, "no terminator here");
    }

    #[test]
    fn flush_returns_all_pending() {
        let mut r = make_renderer();
        r.handle_text_delta("Hello");
        r.handle_block_end_text();
        let result = r.flush();
        assert_eq!(result, "Hello");
        assert!(r.state.pending_text.is_empty());
    }

    #[test]
    fn flush_empty_returns_empty() {
        let mut r = make_renderer();
        let result = r.flush();
        assert!(result.is_empty());
    }

    #[test]
    fn check_timeout_returns_none_when_empty() {
        let mut r = make_renderer();
        assert!(r.check_timeout().is_none());
    }

    #[test]
    fn state_default_values() {
        let state = FeishuStreamingState::default();
        assert!(state.card_id.is_none());
        assert_eq!(state.sequence, 0);
        assert!(!state.is_active);
        assert!(state.last_update.is_none());
        assert!(state.pending_text.is_empty());
    }

    #[test]
    fn state_reset_clears_all() {
        let mut state = FeishuStreamingState::default();
        state.card_id = Some("card_123".to_string());
        state.sequence = 5;
        state.is_active = true;
        state.last_update = Some(Instant::now());
        state.pending_text = "some text".to_string();
        state.reset();
        assert!(state.card_id.is_none());
        assert_eq!(state.sequence, 0);
        assert!(!state.is_active);
        assert!(state.last_update.is_none());
        assert!(state.pending_text.is_empty());
    }

    #[test]
    fn should_update_now_true_when_no_last_update() {
        let r = make_renderer();
        assert!(r.should_update_now());
    }

    #[test]
    fn should_update_now_respects_interval() {
        let mut r = make_renderer();
        r.state.last_update = Some(Instant::now());
        assert!(!r.should_update_now());
    }

    #[test]
    fn streaming_element_id_is_correct() {
        assert_eq!(STREAMING_ELEMENT_ID, "streaming_content");
    }

    #[test]
    fn multiple_text_deltas_accumulate() {
        let mut r = make_renderer();
        r.handle_text_delta("Hello ");
        r.handle_text_delta("world ");
        r.handle_text_delta("!");
        assert!(r.state.pending_text.contains("!"));
    }

    #[test]
    fn code_block_newlines_not_emitted() {
        let mut r = make_renderer();
        r.handle_text_delta("```rust\nfn main() {\n    println!(\"hello\");\n}\n```\n");
        r.handle_block_end_text();
        assert!(r.state.pending_text.contains("fn main()"));
    }
}
