//! Platform-agnostic session router for the unified processor chain.
//!
//! Computes a deterministic [`session_key`] from [`MessageContext::initial_raw()`]
//! fields (`platform`, `sender_id`, `peer_id`) and writes it to the message
//! metadata so that upstream consumers (e.g. [`Gateway::route_message`]) can
//! resolve sessions via [`SessionManager::resolve`].
//!
//! The session key algorithm follows the design doc spec:
//! `session_key = {timestamp_ms}-{sha256(channel:from:to:account_id:timestamp_ms)}`
//!
//! This processor is channel-agnostic — it works for any platform that
//! populates `NormalizedMessage` correctly (terminal, feishu, discord, …).

use async_trait::async_trait;

use closeclaw_gateway::compute_session_key;

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
pub struct SessionRouter;

impl Default for SessionRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionRouter {
    /// Create a new `SessionRouter`.
    pub fn new() -> Self {
        Self
    }

    /// Compute a deterministic session key from routing fields.
    ///
    /// Returns an empty string when `from` or `to` is missing.
    fn compute_key(
        from: &str,
        to: &str,
        channel: &str,
        account_id: Option<&str>,
        timestamp_ms: i64,
    ) -> String {
        if from.is_empty() || to.is_empty() {
            return String::new();
        }
        compute_session_key(channel, from, to, account_id, timestamp_ms)
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
        let session_key = Self::compute_key(
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
