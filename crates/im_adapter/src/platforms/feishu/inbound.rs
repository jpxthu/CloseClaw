//! Inbound normalization — content cleanup, identity mapping, and payload parsing.
//!
//! Extracted from `plugin.rs` to keep impl blocks within the 100-line limit.

use std::time::Instant;

use closeclaw_common::identity::IdentityResolver;
use closeclaw_common::{AdapterError as CommonAdapterError, IMPlugin, NormalizedMessage};
use tracing::debug;

use super::{identity, FeishuPlugin};
use crate::normalized::{add_code_block_language_hint, normalize_urls};
use crate::IMAdapter;

impl FeishuPlugin {
    /// Get the identity resolver for cross-platform account mapping.
    fn identity_resolver(&self) -> Option<&dyn IdentityResolver> {
        self.identity_resolver.as_deref()
    }

    /// Normalize content and apply identity mapping to an inbound message.
    ///
    /// `bot_app_id` is resolved with priority:
    /// 1. `header_app_id` from `last_metadata` (the event header's app_id)
    /// 2. The adapter's own `app_id` (fallback for legacy flows)
    pub(super) fn normalize_inbound_message(&self, msg: &mut NormalizedMessage) {
        msg.content = normalize_urls(&msg.content);
        msg.content = add_code_block_language_hint(&msg.content);
        if let Some(resolver) = self.identity_resolver() {
            let bot_app_id = match self.adapter.last_metadata.try_lock() {
                Ok(guard) => guard
                    .get("header_app_id")
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .unwrap_or_default(),
                Err(_) => {
                    debug!(
                        platform = %msg.platform,
                        sender_id = %msg.sender_id,
                        "try_lock failed, falling back to empty bot_app_id"
                    );
                    String::new()
                }
            };
            msg.account_id = resolver
                .resolve(&msg.platform, &bot_app_id, &msg.sender_id)
                .unwrap_or(std::mem::take(&mut msg.account_id));
        }
    }

    /// Parse an inbound platform payload into a normalized message
    /// (trace_id correlation + identity normalization + debug event).
    ///
    /// Body of `IMPlugin::parse_inbound`, extracted from `plugin.rs` so the
    /// trait impl block stays within the 100-line limit; the trait method
    /// delegates here with an unchanged signature.
    pub(super) async fn parse_inbound_payload(
        &self,
        payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, CommonAdapterError> {
        // Generate trace_id at webhook arrival for cross-chain correlation.
        let trace_id = self.generate_trace_id(self.platform());

        let start = Instant::now();
        let mut msg = self
            .adapter
            .parse_inbound(payload)
            .await
            .map_err(identity::convert_to_common_error)?;
        let parse_duration_ms = start.elapsed().as_millis() as u64;

        // Re-insert trace_id after adapter call — adapter's parse_message_event
        // clears last_metadata and repopulates it with chat_name.
        {
            let mut meta = self.adapter.last_metadata.lock().await;
            meta.insert("trace_id".to_string(), trace_id.clone());
        }

        if let Some(ref mut m) = msg {
            self.normalize_inbound_message(m);
        }

        // Emit structured debug_log event for inbound parse.
        let message_type = msg
            .as_ref()
            .map(|m| {
                serde_json::to_value(&m.message_type)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        self.emit_debug_event(
            "inbound.parse",
            serde_json::json!({
                "platform": "feishu",
                "message_type": message_type,
                "parse_duration_ms": parse_duration_ms,
            }),
        );

        Ok(msg)
    }
}
