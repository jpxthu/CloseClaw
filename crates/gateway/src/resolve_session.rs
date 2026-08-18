//! Session resolution helper for inbound messages.
//!
//! Extracted from `lib.rs` to keep the main file under the 1000-line limit.

use crate::{GatewayConfig, Message};
use closeclaw_common::processor::ProcessedMessage;
use std::collections::HashMap;

impl super::Gateway {
    /// Resolve agent_id from bot→Agent bindings.
    ///
    /// When `peer_id` matches a key in `config.bot_agent_bindings`,
    /// returns the bound agent_id; otherwise returns `peer_id` itself
    /// (backward compatible fallback).
    pub(crate) fn resolve_agent_id(config: &GatewayConfig, peer_id: &str) -> String {
        config
            .bot_agent_bindings
            .get(peer_id)
            .cloned()
            .unwrap_or_else(|| peer_id.to_string())
    }

    /// Resolve a session_id from a [`ProcessedMessage`]'s `session_key`.
    ///
    /// Extracts `session_key` from `metadata` and calls
    /// [`SessionManager::resolve`] to obtain the `session_id`.
    ///
    /// Returns `None` when:
    /// - `session_key` is missing or empty
    /// - [`SessionManager::resolve`] fails
    pub(crate) async fn resolve_session_from_message(
        &self,
        processed: &ProcessedMessage,
        channel: &str,
    ) -> Option<String> {
        let session_key = processed
            .metadata
            .get("session_key")
            .map(|s| s.as_str())
            .unwrap_or("");

        if session_key.is_empty() {
            tracing::warn!("session_key is empty — falling back to routing fields");
        }

        // Build a partial Message for SessionManager::resolve.
        // For existing sessions (key_registry hit), only thread_id is used.
        // For new sessions, to/from are needed for session creation.
        let peer_id = processed
            .metadata
            .get("peer_id")
            .map(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let sender_id = processed
            .metadata
            .get("sender_id")
            .map(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        // Resolve agent_id from bot→Agent bindings.
        // Design doc: "Gateway 根据配置定义的机器人→Agent 绑定确定对应的 Agent".
        let agent_id = Self::resolve_agent_id(&self.config, &peer_id);
        let message = Message {
            id: String::new(),
            from: sender_id,
            to: agent_id,
            content: processed.text_content().unwrap_or("").to_string(),
            channel: channel.to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            metadata: HashMap::new(),
            thread_id: processed.metadata.get("thread_id").cloned(),
            platform: None,
            dsl_result: None,
            content_blocks: None,
        };

        let account_id = processed.metadata.get("account_id").map(|s| s.as_str());

        self.session_manager
            .resolve(session_key, channel, &message, account_id)
            .await
            .ok()
    }
}
