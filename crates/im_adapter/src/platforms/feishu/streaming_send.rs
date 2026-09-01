//! Streaming send helpers — cardkit 3-step protocol and batch send dispatch.
//!
//! Extracted from `mod.rs` to keep file sizes within the 1000-line limit.

use std::sync::Arc;
use std::time::Instant;

use super::cardkit_streaming::CardkitStreamingRenderer;
use super::FeishuAdapter;
use super::FeishuPlugin;
use closeclaw_common::{AdapterError as CommonAdapterError, RenderedOutput};

/// Action returned by [`FeishuPlugin::accumulate_streaming_text`] to indicate
/// what the caller should do next.
enum StreamingAction {
    /// No text to send (empty or not yet time to flush).
    Skip,
    /// Card not yet created — initialize streaming card, then send first update.
    Initialize(String, u32),
    /// Card already exists — send a content update.
    Update(String, String, u32),
}

impl FeishuPlugin {
    /// Create a streaming card via cardkit API and send a reference message.
    ///
    /// Returns `Some(card_id)` on success, `None` on failure.
    pub(super) async fn create_streaming_card_and_ref(
        &self,
        adapter: &FeishuAdapter,
        peer_id: &str,
        root_id: Option<&str>,
    ) -> Option<String> {
        let card_id = CardkitStreamingRenderer::create_card(adapter).await?;
        if let Err(e) =
            CardkitStreamingRenderer::send_card_ref(adapter, peer_id, &card_id, root_id).await
        {
            tracing::warn!(error = %e, card_id = %card_id, "Failed to send streaming card reference");
            return None;
        }
        tracing::info!(card_id = %card_id, peer_id = %peer_id, "Streaming card created and reference sent");
        Some(card_id)
    }

    /// Handle text output during streaming mode via cardkit card updates.
    pub(super) async fn send_streaming_text_with_target(
        &self,
        output: &RenderedOutput,
        peer_id: &str,
        reply_ref: Option<&super::send_helpers::ReplyTarget>,
    ) -> Result<(), CommonAdapterError> {
        let should_batch = {
            let state = self
                .cardkit_streaming
                .lock()
                .expect("cardkit streaming lock poisoned");
            !state.state.is_active
        };
        if should_batch {
            return self
                .send_batch_output_with_target(output, peer_id, reply_ref)
                .await;
        }

        let action = self.accumulate_streaming_text(output);
        match action {
            StreamingAction::Skip => Ok(()),
            StreamingAction::Initialize(content, seq) => {
                self.send_streaming_card_init(&content, seq, peer_id, reply_ref)
                    .await
            }
            StreamingAction::Update(card_id, content, seq) => {
                self.spawn_card_update(self.adapter.clone(), &card_id, &content, seq);
                Ok(())
            }
        }
    }

    /// Accumulate streaming text and determine the next action.
    ///
    /// Handles batch-mode detection, text extraction, pending-text accumulation,
    /// and update-timing checks. Returns a [`StreamingAction`] describing what
    /// the caller should do next.
    fn accumulate_streaming_text(&self, output: &RenderedOutput) -> StreamingAction {
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("");
        if text.is_empty() {
            return StreamingAction::Skip;
        }
        let mut state = self
            .cardkit_streaming
            .lock()
            .expect("cardkit streaming lock poisoned");
        state.state.pending_text.push_str(text);
        if !state.should_update_now() {
            return StreamingAction::Skip;
        }
        let content = std::mem::take(&mut state.state.pending_text);
        state.state.sequence += 1;
        let seq = state.state.sequence;
        state.state.last_update = Some(Instant::now());
        if state.state.card_id.is_none() {
            StreamingAction::Initialize(content, seq)
        } else {
            let card_id = state
                .state
                .card_id
                .clone()
                .expect("card_id must be set after creation");
            StreamingAction::Update(card_id, content, seq)
        }
    }

    /// Initialize the streaming card (create + send ref) and send first update.
    #[allow(clippy::too_many_lines)]
    async fn send_streaming_card_init(
        &self,
        content: &str,
        seq: u32,
        peer_id: &str,
        reply_ref: Option<&super::send_helpers::ReplyTarget>,
    ) -> Result<(), CommonAdapterError> {
        let adapter = self.adapter.clone();
        let peer = peer_id.to_string();
        // For cardkit, extract root_id from ReplyTarget::Thread for thread routing
        let root = match reply_ref {
            Some(super::send_helpers::ReplyTarget::Thread { root_id }) => Some(root_id.clone()),
            _ => None,
        };
        let content = content.to_string();
        let card_id = self
            .create_streaming_card_and_ref(&adapter, &peer, root.as_deref())
            .await;
        let card_id = match card_id {
            Some(id) => id,
            None => return Ok(()),
        };
        {
            let mut state = self
                .cardkit_streaming
                .lock()
                .expect("cardkit streaming lock poisoned");
            state.state.card_id = Some(card_id.clone());
        }
        self.spawn_card_update(adapter, &card_id, &content, seq);
        Ok(())
    }

    /// Spawn an async task to update a card element.
    fn spawn_card_update(
        &self,
        adapter: Arc<FeishuAdapter>,
        card_id: &str,
        content: &str,
        seq: u32,
    ) {
        let card_id = card_id.to_string();
        let content = content.to_string();
        let element_id = super::cardkit_streaming::STREAMING_ELEMENT_ID.to_string();
        tokio::spawn(async move {
            if let Err(e) = CardkitStreamingRenderer::update_element(
                &adapter,
                &card_id,
                &element_id,
                &content,
                seq,
            )
            .await
            {
                tracing::warn!(error = %e, card_id = %card_id, "Failed to update streaming card element");
            }
        });
    }

    /// Send a batch output (non-streaming path).
    pub(super) async fn send_batch_output_with_target(
        &self,
        output: &RenderedOutput,
        peer_id: &str,
        reply_ref: Option<&super::send_helpers::ReplyTarget>,
    ) -> Result<(), CommonAdapterError> {
        let msg_type = output.msg_type.clone();
        let start = Instant::now();
        let result = self.dispatch_send(peer_id, output, reply_ref).await;
        let send_duration_ms = start.elapsed().as_millis() as u64;
        let success = result.is_ok();
        self.emit_debug_event(
            "outbound.send",
            serde_json::json!({
                "platform": "feishu",
                "peer_id": peer_id,
                "msg_type": msg_type,
                "send_duration_ms": send_duration_ms,
                "success": success,
            }),
        );
        result
    }
}
