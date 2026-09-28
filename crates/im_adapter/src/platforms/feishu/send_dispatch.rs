//! Send dispatch — route rendered output to the Feishu send API.
//!
//! Extracted from `plugin.rs` to keep impl blocks within the 100-line limit.

use closeclaw_common::{AdapterError as CommonAdapterError, RenderedOutput};
use tracing::warn;

use super::card_media_fallback;
use super::send_helpers;
use super::FeishuPlugin;

impl FeishuPlugin {
    /// Fallback: extract plain text and media from an interactive card
    /// and send them via text message API and `dispatch_send_media`.
    /// Logs warnings on failure and always returns so the Agent keeps running.
    async fn send_interactive_fallback(
        &self,
        peer_id: &str,
        output: &RenderedOutput,
        reply_ref: Option<&send_helpers::ReplyTarget>,
    ) {
        card_media_fallback::send_interactive_fallback(&self.adapter, peer_id, output, reply_ref)
            .await
    }

    /// Dispatch a rendered output to the platform send API.
    pub(super) async fn dispatch_send(
        &self,
        peer_id: &str,
        output: &RenderedOutput,
        reply_ref: Option<&send_helpers::ReplyTarget>,
    ) -> Result<(), CommonAdapterError> {
        match output.msg_type.as_str() {
            "text" => {
                let text = output
                    .payload
                    .get("content")
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                if let Err(e) = self
                    .adapter
                    .send_msg(peer_id, "text", text, reply_ref)
                    .await
                {
                    warn!(peer_id = %peer_id, error = %e,
                        "Feishu text send failed — returning Ok(()) per design doc");
                }
                Ok(())
            }
            "interactive" => {
                // Process media elements in the card payload before sending.
                let mut payload = output.payload.clone();
                if let Err(e) = self.process_card_media(&mut payload).await {
                    warn!(peer_id = %peer_id, error = %e,
                        "Failed to process card media — sending as-is");
                }
                let card_json = serde_json::to_string(&payload)
                    .map_err(|e| CommonAdapterError::SendFailed(e.to_string()))?;
                match self
                    .adapter
                    .send_msg(peer_id, "interactive", &card_json, reply_ref)
                    .await
                {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        warn!(peer_id = %peer_id, error = %e,
                            "Feishu interactive card send failed — falling back to text + media");
                        self.send_interactive_fallback(peer_id, output, reply_ref)
                            .await;
                        Ok(())
                    }
                }
            }
            _ => Err(CommonAdapterError::UnsupportedOperation),
        }
    }
}
