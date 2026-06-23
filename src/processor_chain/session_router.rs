//! Platform-agnostic session router for the unified processor chain.
//!
//! Computes a deterministic [`session_key`](DmScope::compute_session_key)
//! from [`MessageContext::initial_raw()`] fields (`platform`, `sender_id`,
//! `peer_id`) and writes it to the message metadata so that upstream
//! consumers (e.g. [`Gateway::route_message`]) can resolve sessions via
//! [`SessionManager::resolve`].
//!
//! By default uses [`DmScope::PerAccountChannelPeer`] mode, producing
//! keys in the format `{account_id}:{platform}:{sender_id}:{peer_id}`.
//! The scope can be overridden via the [`DmScope`] configuration.
//!
//! This processor is channel-agnostic — it works for any platform that
//! populates `RawMessage` correctly (terminal, feishu, discord, …).

use async_trait::async_trait;

use crate::gateway::{DmScope, Message};

use super::context::MessageContext;
use super::error::ProcessError;
use super::processor::{MessageProcessor, ProcessPhase};
use super::ProcessedMessage;

/// Inbound processor that computes and attaches a `session_key` to the
/// message metadata.
///
/// Runs at priority 20 — after [`RawLogProcessor`](super::raw_log_processor)
/// (10) and before [`ContentNormalizer`](super::content_normalizer) (30).
#[derive(Debug, Clone)]
pub struct SessionRouter {
    dm_scope: DmScope,
}

impl SessionRouter {
    /// Create a new `SessionRouter` with the given [`DmScope`].
    pub fn new(dm_scope: DmScope) -> Self {
        Self { dm_scope }
    }

    /// Compute a deterministic session key from routing fields.
    ///
    /// Returns an empty string when `from` or `to` is missing.
    fn compute_key(&self, from: &str, to: &str, channel: &str, account_id: Option<&str>) -> String {
        if from.is_empty() || to.is_empty() {
            return String::new();
        }
        let msg = Message {
            id: String::new(),
            from: from.to_string(),
            to: to.to_string(),
            content: String::new(),
            channel: channel.to_string(),
            timestamp: 0,
            metadata: std::collections::HashMap::new(),
            thread_id: None,
        };
        self.dm_scope.compute_session_key(channel, &msg, account_id)
    }
}

#[async_trait]
impl MessageProcessor for SessionRouter {
    fn name(&self) -> &str {
        "SessionRouter"
    }

    fn phase(&self) -> ProcessPhase {
        ProcessPhase::Inbound
    }

    fn priority(&self) -> u8 {
        20
    }

    async fn process(
        &self,
        ctx: &MessageContext,
    ) -> Result<Option<ProcessedMessage>, ProcessError> {
        let raw = ctx
            .initial_raw()
            .cloned()
            .unwrap_or_else(|| super::context::RawMessage {
                platform: String::new(),
                sender_id: String::new(),
                peer_id: String::new(),
                content: ctx.content.clone(),
                timestamp: chrono::Utc::now(),
                message_id: String::new(),
            });

        let platform = raw.platform;
        let sender_id = raw.sender_id;
        let peer_id = raw.peer_id;
        let account_id = ctx
            .metadata
            .get("account_id")
            .and_then(|v| v.as_str().map(String::from));

        let session_key = self.compute_key(&sender_id, &peer_id, &platform, account_id.as_deref());

        let mut metadata = ctx.metadata.clone();
        if !session_key.is_empty() {
            metadata.insert(
                "session_key".to_string(),
                serde_json::Value::String(session_key),
            );
        }
        metadata.insert("platform".to_string(), serde_json::Value::String(platform));
        metadata.insert(
            "sender_id".to_string(),
            serde_json::Value::String(sender_id),
        );
        metadata.insert("peer_id".to_string(), serde_json::Value::String(peer_id));

        Ok(Some(ProcessedMessage {
            content: ctx.content.clone(),
            metadata,
            suppress: false,
            content_blocks: vec![],
        }))
    }
}

#[cfg(test)]
#[path = "session_router_tests.rs"]
mod tests;
