//! Session resolution helper for inbound messages.
//!
//! Extracted from `lib.rs` to keep the main file under the 1000-line limit.

use crate::Message;
use closeclaw_common::processor::ProcessedMessage;
use std::collections::HashMap;

impl super::Gateway {
    /// Resolve agent_id from bot→Agent bindings.
    ///
    /// When `peer_id` matches a key in `bindings`,
    /// returns the bound agent_id; otherwise returns `peer_id` itself
    /// (backward compatible fallback).
    pub(crate) fn resolve_agent_id(bindings: &HashMap<String, String>, peer_id: &str) -> String {
        bindings
            .get(peer_id)
            .cloned()
            .unwrap_or_else(|| peer_id.to_string())
    }

    /// Build a [`Message`] from [`ProcessedMessage`] metadata for session resolution.
    ///
    /// Resolves the target agent_id (see [`select_agent_id`] for the
    /// priority rules), then constructs a partial Message
    /// suitable for [`SessionManager::resolve`].
    fn build_resolve_message(
        processed: &ProcessedMessage,
        channel: &str,
        bindings: &HashMap<String, String>,
    ) -> Message {
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
        let agent_id = select_agent_id(&processed.metadata, bindings, &peer_id);
        Message {
            id: String::new(),
            from: sender_id,
            to: agent_id,
            content: processed.text_content().unwrap_or("").to_string(),
            channel: channel.to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            metadata: HashMap::new(),
            thread_id: processed.metadata.get("thread_id").cloned(),
            reply_ref: processed.metadata.get("reply_ref").cloned(),
            platform: None,
            dsl_result: None,
            content_blocks: None,
        }
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

        let message =
            Self::build_resolve_message(processed, channel, &self.config.bot_agent_bindings);
        let account_id = processed.metadata.get("account_id").map(|s| s.as_str());
        // Pass agent_id explicitly per design doc: Gateway resolves agent_id
        // from bot→Agent bindings and passes it as an independent parameter.
        let agent_id = message.to.as_str();

        self.session_manager
            .resolve(session_key, channel, &message, account_id, agent_id)
            .await
            .ok()
    }
}

/// Select the target agent_id for an inbound message.
///
/// Agent resolution priority:
/// 1. Explicit `agent_id` in message metadata — set by channels where
///    the user names the target agent directly (design doc
///    `cli/chat.md`: "用户通过 --agent-id 指定目标 agent").
/// 2. bot→Agent bindings lookup on `peer_id`.
/// 3. `peer_id` itself (backward-compatible fallback).
fn select_agent_id(
    metadata: &HashMap<String, String>,
    bindings: &HashMap<String, String>,
    peer_id: &str,
) -> String {
    // Explicit request target (e.g. terminal chat `--agent-id`) wins;
    // otherwise design doc: "Gateway 根据配置定义的机器人→Agent 绑定
    // 确定对应的 Agent".
    match metadata.get("agent_id").filter(|s| !s.is_empty()).cloned() {
        Some(explicit) => {
            tracing::info!(
                agent_id = %explicit,
                peer_id = %peer_id,
                "routing by explicit request agent_id"
            );
            explicit
        }
        None => super::Gateway::resolve_agent_id(bindings, peer_id),
    }
}

#[cfg(test)]
#[path = "resolve_session_tests.rs"]
mod tests;
