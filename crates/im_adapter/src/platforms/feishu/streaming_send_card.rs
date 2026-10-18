//! Streaming card protocol — card creation, init, and async element updates.
//!
//! Extracted from `streaming_send.rs` to keep impl blocks within the 100-line limit.

use std::sync::Arc;

use super::cardkit_streaming::CardkitStreamingRenderer;
use super::cardkit_streaming::STREAMING_ELEMENT_ID;
use super::send_helpers::ReplyTarget;
use super::FeishuAdapter;
use super::FeishuPlugin;
use closeclaw_common::AdapterError as CommonAdapterError;

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
            tracing::warn!(
                error = %e,
                card_id = %card_id,
                "Failed to send streaming card reference"
            );
            return None;
        }
        tracing::info!(
            card_id = %card_id,
            peer_id = %peer_id,
            "Streaming card created and reference sent"
        );
        Some(card_id)
    }

    /// Initialize the streaming card (create + send ref) and send first update.
    pub(super) async fn send_streaming_card_init(
        &self,
        content: &str,
        seq: u32,
        peer_id: &str,
        reply_ref: Option<&ReplyTarget>,
    ) -> Result<(), CommonAdapterError> {
        let adapter = self.adapter.clone();
        let peer = peer_id.to_string();
        // For cardkit, extract root_id from ReplyTarget::Thread for thread routing
        let root = match reply_ref {
            Some(ReplyTarget::Thread { root_id }) => Some(root_id.clone()),
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
            let mut state = self.lock_streaming_state();
            state.state.card_id = Some(card_id.clone());
        }
        self.spawn_card_update(adapter, &card_id, &content, seq);
        Ok(())
    }

    /// Spawn an async task to update a card element.
    pub(super) fn spawn_card_update(
        &self,
        adapter: Arc<FeishuAdapter>,
        card_id: &str,
        content: &str,
        seq: u32,
    ) {
        let card_id = card_id.to_string();
        let content = content.to_string();
        let element_id = STREAMING_ELEMENT_ID.to_string();
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
                tracing::warn!(
                    error = %e,
                    card_id = %card_id,
                    "Failed to update streaming card element"
                );
            }
        });
    }
}
