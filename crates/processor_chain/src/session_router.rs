//! Platform-agnostic session router for the unified processor chain.
//!
//! Computes a deterministic [`session_key`](DmScope::compute_session_key)
//! from [`MessageContext::initial_raw()`] fields (`platform`, `sender_id`,
//! `peer_id`) and writes it to the message metadata so that upstream
//! consumers (e.g. [`Gateway::route_message`]) can resolve sessions via
//! [`SessionManager::resolve`].
//!
//! By default uses [`DmScope::PerAccountChannelPeer`] mode, producing
//! keys in the format `{platform}:{sender_id}:{peer_id}:{account_id}`.
//! The scope can be overridden via the [`DmScope`] configuration.
//!
//! This processor is channel-agnostic — it works for any platform that
//! populates `NormalizedMessage` correctly (terminal, feishu, discord, …).

use async_trait::async_trait;

use closeclaw_gateway::{DmScope, Message};

use super::context::MessageContext;
use super::error::ProcessError;
use super::processor::{MessageProcessor, ProcessPhase};
use super::ProcessedMessage;
use closeclaw_llm::types::ContentBlock;

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
    fn compute_key(
        &self,
        from: &str,
        to: &str,
        channel: &str,
        account_id: Option<&str>,
        timestamp_ms: i64,
    ) -> String {
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
        self.dm_scope
            .compute_session_key(channel, &msg, account_id, timestamp_ms)
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
        let msg = ctx.initial_normalized().cloned().unwrap_or_else(|| {
            closeclaw_common::im_plugin::NormalizedMessage {
                platform: String::new(),
                sender_id: String::new(),
                peer_id: String::new(),
                content: ctx.content.clone(),
                timestamp: chrono::Utc::now().timestamp_millis(),
                message_type: Default::default(),
                media_refs: Vec::new(),
                thread_id: None,
                account_id: String::new(),
            }
        });

        let platform = msg.platform;
        let sender_id = msg.sender_id;
        let peer_id = msg.peer_id;
        let account_id = if msg.account_id.is_empty() {
            None
        } else {
            Some(msg.account_id.clone())
        };

        // Use system time instead of message timestamp to align with design doc:
        // "timestamp_ms 为当前系统时间的毫秒级时间戳"
        let timestamp_ms = chrono::Utc::now().timestamp_millis();
        let session_key = self.compute_key(
            &sender_id,
            &peer_id,
            &platform,
            account_id.as_deref(),
            timestamp_ms,
        );

        if session_key.is_empty() {
            let mut missing = Vec::new();
            if sender_id.is_empty() {
                missing.push("from");
            }
            if peer_id.is_empty() {
                missing.push("to");
            }
            tracing::warn!(
                platform = %platform,
                missing_fields = ?missing,
                "SessionRouter: session_key computation failed, leaving key empty"
            );
        }

        let mut metadata = ctx.metadata.clone();
        if !session_key.is_empty() {
            metadata.insert("session_key".to_string(), session_key);
        }
        metadata.insert("platform".to_string(), platform);
        metadata.insert("sender_id".to_string(), sender_id);
        metadata.insert("peer_id".to_string(), peer_id);

        Ok(Some(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text(ctx.content.clone())],
            metadata,
        }))
    }
}

#[cfg(test)]
#[path = "session_router_tests.rs"]
mod tests;
