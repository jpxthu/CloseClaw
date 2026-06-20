//! Platform-agnostic session router for the unified processor chain.
//!
//! Computes a deterministic [`session_key`](DmScope::compute_session_key)
//! from [`MessageContext::initial_raw()`] fields (`platform`, `sender_id`,
//! `peer_id`) and writes it to the message metadata so that upstream
//! consumers (e.g. [`Gateway::route_message`]) can resolve sessions via
//! [`SessionManager::resolve`].
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

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processor_chain::context::RawMessage;

    fn make_router(dm_scope: DmScope) -> SessionRouter {
        SessionRouter::new(dm_scope)
    }

    fn make_ctx(raw: RawMessage) -> MessageContext {
        MessageContext::from_raw(raw)
    }

    #[tokio::test]
    async fn test_terminal_session_key_computed() {
        let router = make_router(DmScope::PerChannelPeer);
        let raw = RawMessage {
            platform: "terminal".to_string(),
            sender_id: "1000".to_string(),
            peer_id: "cli".to_string(),
            content: "hello".to_string(),
            timestamp: chrono::Utc::now(),
            message_id: "msg_1".to_string(),
        };
        let ctx = make_ctx(raw);
        let result = router.process(&ctx).await.unwrap().unwrap();
        let key = result
            .metadata
            .get("session_key")
            .and_then(|v| v.as_str())
            .unwrap();
        assert!(!key.is_empty(), "session_key should not be empty");
        assert!(key.contains("1000"), "key should contain sender_id: {key}");
        assert!(key.contains("cli"), "key should contain peer_id: {key}");
        assert!(
            key.contains("terminal"),
            "key should contain platform: {key}"
        );
    }

    #[tokio::test]
    async fn test_deterministic_key() {
        let router = make_router(DmScope::PerAccountChannelPeer);
        let raw = RawMessage {
            platform: "feishu".to_string(),
            sender_id: "ou_abc".to_string(),
            peer_id: "oc_xyz".to_string(),
            content: "hi".to_string(),
            timestamp: chrono::Utc::now(),
            message_id: "msg_d".to_string(),
        };
        let ctx = make_ctx(raw);
        let r1 = router.process(&ctx).await.unwrap().unwrap();
        let r2 = router.process(&ctx).await.unwrap().unwrap();
        let k1 = r1.metadata.get("session_key").unwrap();
        let k2 = r2.metadata.get("session_key").unwrap();
        assert_eq!(k1, k2, "same input must produce same session_key");
    }

    #[tokio::test]
    async fn test_missing_peer_id_yields_empty_key() {
        let router = make_router(DmScope::PerChannelPeer);
        let raw = RawMessage {
            platform: "terminal".to_string(),
            sender_id: "u1".to_string(),
            peer_id: String::new(),
            content: "hi".to_string(),
            timestamp: chrono::Utc::now(),
            message_id: "msg_e".to_string(),
        };
        let ctx = make_ctx(raw);
        let result = router.process(&ctx).await.unwrap().unwrap();
        let key = result
            .metadata
            .get("session_key")
            .map(|v| v.as_str().unwrap_or(""))
            .unwrap_or("");
        assert!(key.is_empty(), "missing peer_id should yield empty key");
    }

    #[tokio::test]
    async fn test_dm_scope_affects_key() {
        let r1 = make_router(DmScope::PerPeer);
        let r2 = make_router(DmScope::PerChannelPeer);
        let raw = RawMessage {
            platform: "discord".to_string(),
            sender_id: "user_1".to_string(),
            peer_id: "dm_42".to_string(),
            content: "test".to_string(),
            timestamp: chrono::Utc::now(),
            message_id: "msg_f".to_string(),
        };
        let ctx = make_ctx(raw);
        let k1 = r1
            .process(&ctx)
            .await
            .unwrap()
            .unwrap()
            .metadata
            .get("session_key")
            .unwrap()
            .clone();
        let k2 = r2
            .process(&ctx)
            .await
            .unwrap()
            .unwrap()
            .metadata
            .get("session_key")
            .unwrap()
            .clone();
        assert_ne!(k1, k2, "different DmScope should produce different keys");
    }

    #[tokio::test]
    async fn test_metadata_preserves_upstream() {
        let router = make_router(DmScope::PerChannelPeer);
        let raw = RawMessage {
            platform: "terminal".to_string(),
            sender_id: "1000".to_string(),
            peer_id: "cli".to_string(),
            content: "hi".to_string(),
            timestamp: chrono::Utc::now(),
            message_id: "msg_g".to_string(),
        };
        let mut ctx = make_ctx(raw);
        ctx.metadata.insert(
            "existing_key".to_string(),
            serde_json::json!("existing_value"),
        );
        let result = router.process(&ctx).await.unwrap().unwrap();
        assert_eq!(
            result.metadata.get("existing_key").unwrap(),
            "existing_value"
        );
        assert!(result.metadata.contains_key("session_key"));
        assert!(result.metadata.contains_key("platform"));
        assert!(result.metadata.contains_key("sender_id"));
        assert!(result.metadata.contains_key("peer_id"));
    }

    #[tokio::test]
    async fn test_fallback_when_no_initial_raw() {
        let router = make_router(DmScope::PerChannelPeer);
        let raw = RawMessage {
            platform: String::new(),
            sender_id: String::new(),
            peer_id: String::new(),
            content: String::new(),
            timestamp: chrono::Utc::now(),
            message_id: String::new(),
        };
        let ctx = MessageContext::from_raw(raw);
        let result = router.process(&ctx).await.unwrap().unwrap();
        // No initial_raw → fallback raw with empty fields → empty session_key
        assert!(
            !result.metadata.contains_key("session_key"),
            "no key when raw is absent"
        );
    }
}
