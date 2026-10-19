//! Plain-text fallbacks for the batch outbound path: log-only fallback
//! when no IM plugin is registered for the channel, and plain-text
//! re-send when render/send fails.

use crate::{Gateway, GatewayError};
use closeclaw_common::im_plugin::{IMPlugin, RenderedOutput};
use closeclaw_llm::types::ContentBlock;
use std::sync::Arc;

impl Gateway {
    /// Fallback to plain-text output when no IM plugin is registered for
    /// the target channel. Logs a warning, records the raw text to the
    /// outbound log (via `process_outbound_raw_log_only`), and returns `Ok(())`
    /// so the caller does not fail.
    pub(crate) async fn fallback_to_plain_text(
        &self,
        channel: &str,
        raw_output: &str,
    ) -> Result<(), GatewayError> {
        tracing::warn!(
            channel,
            "no IM plugin registered, falling back to plain-text log"
        );
        let blocks = vec![ContentBlock::Text(raw_output.to_string())];
        self.process_outbound_raw_log_only(raw_output, blocks, channel)
            .await?;
        Ok(())
    }

    /// Fallback to plain-text send when render/send fails.
    #[allow(dead_code)] // kept for test assertions; no longer called in production path
    pub(crate) async fn send_as_plain_text(
        &self,
        plugin: &Arc<dyn IMPlugin>,
        raw_output: &str,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), GatewayError> {
        tracing::warn!(
            chat_id,
            "render/send failed, falling back to plain-text send"
        );
        let rendered = RenderedOutput {
            msg_type: "text".to_string(),
            payload: serde_json::json!({ "content": { "text": raw_output } }),
        };
        plugin.send(&rendered, chat_id, thread_id, None).await?;
        Ok(())
    }
}
