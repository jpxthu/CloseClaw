//! Inbound normalization — content cleanup and identity mapping.
//!
//! Extracted from `plugin.rs` to keep impl blocks within the 100-line limit.

use closeclaw_common::identity::IdentityResolver;
use closeclaw_common::NormalizedMessage;
use tracing::debug;

use super::FeishuPlugin;
use crate::normalized::{add_code_block_language_hint, normalize_urls};

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
}
