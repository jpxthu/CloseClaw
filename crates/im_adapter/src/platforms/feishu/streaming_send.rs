//! Streaming send entry — batch/streaming routing and text accumulation.
//!
//! Extracted from `mod.rs` to keep file sizes within the 1000-line limit.

use std::time::Instant;

use super::cardkit_streaming::CardkitStreamingRenderer;
use super::FeishuPlugin;
use closeclaw_common::{AdapterError as CommonAdapterError, RenderedOutput};

/// Action returned by [`FeishuPlugin::accumulate_streaming_text`] to indicate
/// what the caller should do next.
pub(super) enum StreamingAction {
    /// No text to send (empty or not yet time to flush).
    Skip,
    /// Card not yet created — initialize streaming card, then send first update.
    Initialize(String, u32),
    /// Card already exists — send a content update.
    Update(String, String, u32),
}

impl FeishuPlugin {
    /// Lock the cardkit streaming state, panicking if the lock is poisoned.
    pub(super) fn lock_streaming_state(
        &self,
    ) -> std::sync::MutexGuard<'_, CardkitStreamingRenderer> {
        self.cardkit_streaming
            .lock()
            .expect("cardkit streaming lock poisoned")
    }

    /// Handle text output during streaming mode via cardkit card updates.
    pub(super) async fn send_streaming_text_with_target(
        &self,
        output: &RenderedOutput,
        peer_id: &str,
        reply_ref: Option<&super::send_helpers::ReplyTarget>,
    ) -> Result<(), CommonAdapterError> {
        let should_batch = {
            let state = self.lock_streaming_state();
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
    pub(super) fn accumulate_streaming_text(&self, output: &RenderedOutput) -> StreamingAction {
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("");
        if text.is_empty() {
            return StreamingAction::Skip;
        }
        let mut state = self.lock_streaming_state();
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
}
