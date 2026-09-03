//! Feishu cardkit streaming renderer.
//!
//! Implements the three-step cardkit protocol for streaming LLM output:
//! 1. Create a streaming card (`POST /cardkit/v1/cards`, `streaming_mode: true`)
//! 2. Send a card-reference message (card_json with `card_id`)
//! 3. Batch-update card element content (`PUT /cardkit/v1/cards/{card_id}/elements/{element_id}/content`)
//!
//! Text line-buffering is delegated to [`DefaultStreamingRenderer`]
//! (via [`super::FeishuPlugin::streaming_renderer`]). This module
//! is only responsible for the cardkit protocol layer: card creation,
//! reference sending, element updates, update frequency limiting,
//! and the `pending_text` buffer for batch cardkit updates.

use std::time::{Duration, Instant};

use super::adapter::FeishuAdapter;
use super::send_helpers::run_cli;

/// Cardkit streaming element ID for the main content element.
pub(crate) const STREAMING_ELEMENT_ID: &str = "streaming_content";

/// Streaming card state managed per-response.
#[derive(Default)]
pub(crate) struct FeishuStreamingState {
    /// Card ID assigned by cardkit on creation.
    pub(crate) card_id: Option<String>,
    /// Monotonically increasing sequence for card updates.
    pub(crate) sequence: u32,
    /// Whether streaming is active (card created, updates in progress).
    pub(crate) is_active: bool,
    /// Timestamp of the last card update sent to the platform.
    pub(crate) last_update: Option<Instant>,
    /// Accumulated text since the last card update.
    ///
    /// Fed by `FeishuPlugin::handle_stream_event` / `flush_stream` /
    /// `check_stream_timeout` which route text from the default streaming
    /// renderer to this buffer for batch cardkit updates.
    pub(crate) pending_text: String,
}

impl FeishuStreamingState {
    /// Reset all streaming state for a new response.
    #[allow(dead_code)]
    pub(crate) fn reset(&mut self) {
        self.card_id = None;
        self.sequence = 0;
        self.is_active = false;
        self.last_update = None;
        self.pending_text.clear();
    }
}

/// Feishu cardkit streaming renderer.
///
/// Manages the cardkit three-step protocol (create → send ref → batch update).
/// Text line-buffering is handled by the default streaming renderer in
/// `FeishuPlugin`; this renderer only accumulates the resulting text lines
/// in `pending_text` for batch cardkit element updates.
pub(crate) struct CardkitStreamingRenderer {
    pub(crate) state: FeishuStreamingState,
}

impl CardkitStreamingRenderer {
    /// Create a new cardkit streaming renderer.
    pub(crate) fn new() -> Self {
        Self {
            state: FeishuStreamingState::default(),
        }
    }

    /// Check if a card update should be sent based on timing.
    ///
    /// Enforces a minimum interval between card updates to avoid
    /// exceeding the platform's update frequency limit.
    pub(crate) fn should_update_now(&self) -> bool {
        match self.state.last_update {
            Some(last) => last.elapsed() >= Duration::from_millis(100),
            None => true,
        }
    }

    /// Return the current pending text buffer (test-only).
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn pending_text(&self) -> &str {
        &self.state.pending_text
    }

    /// Set card_id for testing cardkit state transitions.
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn set_card_id_for_test(&mut self, card_id: &str) {
        self.state.card_id = Some(card_id.to_string());
    }

    /// Create a streaming card via cardkit API.
    ///
    /// Returns `Some(card_id)` on success, `None` on failure.
    pub(crate) async fn create_card(adapter: &FeishuAdapter) -> Option<String> {
        let card_json = serde_json::json!({
            "streaming_mode": true,
            "body": {
                "elements": [{
                    "tag": "markdown",
                    "element_id": STREAMING_ELEMENT_ID,
                    "content": ""
                }]
            }
        });

        let data_str = serde_json::json!({
            "type": "card_json",
            "data": card_json.to_string()
        })
        .to_string();

        let args = [
            "api",
            "--method",
            "POST",
            "--uri",
            "/open-apis/cardkit/v1/cards",
            "--data",
            &data_str,
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
    ///
    /// Routes to `--user-id` for P2P (`ou_xxx`) or `--chat-id` for
    /// group chats (`oc_xxx`).
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
        let reply_ref = root_id.map(|id| super::send_helpers::ReplyTarget::Thread {
            root_id: id.to_string(),
        });
        adapter
            .send_msg(chat_id, "interactive", &content, reply_ref.as_ref())
            .await
    }

    /// Update a card element's content via cardkit API.
    ///
    /// Sends a PUT request to update the markdown content of a specific
    /// element within a streaming card. The sequence number must be
    /// monotonically increasing for idempotent updates. A `uuid` field
    /// (derived from sequence + element_id) ensures deduplication.
    pub(crate) async fn update_element(
        adapter: &FeishuAdapter,
        card_id: &str,
        element_id: &str,
        content: &str,
        sequence: u32,
    ) -> Result<(), crate::error::AdapterError> {
        let uuid = format!("{}_{}", sequence, element_id);
        let update_json = serde_json::json!({
            "content": content,
            "uuid": uuid,
            "sequence": sequence
        });
        let update_str = update_json.to_string();

        let uri = format!(
            "/open-apis/cardkit/v1/cards/{}/elements/{}/content",
            card_id, element_id
        );

        let args = [
            "api",
            "--method",
            "PUT",
            "--uri",
            &uri,
            "--data",
            &update_str,
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

    /// Simulate text routed from DefaultStreamingRenderer into cardkit
    /// pending_text (the new data flow after Step 1.1).
    fn push_text(r: &mut CardkitStreamingRenderer, text: &str) {
        r.state.pending_text.push_str(text);
    }

    // =========================================================================
    // pending_text accumulation (text routed from DefaultStreamingRenderer)
    // =========================================================================

    #[test]
    fn pending_text_direct_accumulation() {
        let mut r = make_renderer();
        push_text(&mut r, "Hello ");
        push_text(&mut r, "world!");
        assert_eq!(r.state.pending_text, "Hello world!");
    }

    #[test]
    fn pending_text_preserves_all_content() {
        let mut r = make_renderer();
        push_text(&mut r, "Hello world! ");
        assert_eq!(r.state.pending_text, "Hello world! ");
    }

    #[test]
    fn pending_text_with_newline() {
        let mut r = make_renderer();
        push_text(&mut r, "line1\nline2");
        assert_eq!(r.state.pending_text, "line1\nline2");
    }

    #[test]
    fn pending_text_without_terminator_preserved() {
        let mut r = make_renderer();
        push_text(&mut r, "no terminator here");
        assert_eq!(r.state.pending_text, "no terminator here");
    }

    #[test]
    fn pending_text_empty_initially() {
        let r = make_renderer();
        assert!(r.pending_text().is_empty());
    }

    #[test]
    fn pending_text_cleared_by_reset() {
        let mut r = make_renderer();
        push_text(&mut r, "some content。");
        assert!(!r.state.pending_text.is_empty());
        r.state.reset();
        assert!(r.pending_text().is_empty());
    }

    // =========================================================================
    // State defaults and reset
    // =========================================================================

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

    // =========================================================================
    // Cardkit protocol state tests
    // =========================================================================

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
    fn sequence_starts_at_zero() {
        let state = FeishuStreamingState::default();
        assert_eq!(state.sequence, 0);
    }

    #[test]
    fn sequence_increments_on_update() {
        let mut state = FeishuStreamingState::default();
        state.sequence += 1;
        assert_eq!(state.sequence, 1);
        state.sequence += 1;
        assert_eq!(state.sequence, 2);
    }

    #[test]
    fn card_id_set_during_streaming() {
        let mut state = FeishuStreamingState::default();
        assert!(state.card_id.is_none());
        state.card_id = Some("card_abc123".to_string());
        assert_eq!(state.card_id.as_deref(), Some("card_abc123"));
    }

    #[test]
    fn streaming_active_flag_toggled() {
        let mut state = FeishuStreamingState::default();
        assert!(!state.is_active);
        state.is_active = true;
        assert!(state.is_active);
        state.is_active = false;
        assert!(!state.is_active);
    }

    // =========================================================================
    // Reset during active streaming
    // =========================================================================

    #[test]
    fn reset_during_active_streaming_clears_state() {
        let mut r = make_renderer();
        push_text(&mut r, "partial content。");
        r.state.card_id = Some("active_card".to_string());
        r.state.sequence = 3;
        r.state.is_active = true;
        r.state.reset();
        assert!(r.state.card_id.is_none());
        assert_eq!(r.state.sequence, 0);
        assert!(!r.state.is_active);
        assert!(r.state.pending_text.is_empty());
    }
}
