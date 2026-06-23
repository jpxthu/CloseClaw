//! SessionRouter — inbound MessageProcessor that computes session keys.
//!
//! Extracts routing fields (`account_id`, `from`, `to`, `channel`)
//! directly from the raw webhook JSON, then computes a deterministic
//! `session_key` via [`DmScope::compute_session_key`] and writes it
//! to the message metadata.
//!
//! By default uses [`DmScope::PerAccountChannelPeer`] mode, producing
//! keys in the format `{account_id}:{channel}:{from}:{to}`.
//!
//! Runs at priority 20, before [`FeishuMessageCleaner`] (priority 30).

use super::{MessageContext, MessageProcessor, ProcessError, ProcessPhase, ProcessedMessage};
use crate::gateway::{DmScope, Message};
use async_trait::async_trait;
use serde_json::Value;

/// SessionRouter — computes and attaches a session_key to the message pipeline.
#[derive(Debug, Clone)]
pub struct SessionRouter {
    dm_scope: DmScope,
}

impl SessionRouter {
    /// Create a new SessionRouter with the given [`DmScope`].
    pub fn new(dm_scope: DmScope) -> Self {
        Self { dm_scope }
    }

    /// Extract feishu sender open_id from the raw webhook.
    fn extract_from(raw: &Value) -> Result<String, ProcessError> {
        raw.get("sender")
            .and_then(|s| s.get("sender_id"))
            .and_then(|sid| sid.get("open_id"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| {
                ProcessError::ProcessingFailed(
                    "SessionRouter: cannot extract 'from' (sender.open_id)".into(),
                )
            })
    }

    /// Extract feishu chat_id as the `to` / agent_id from the raw webhook.
    fn extract_to(raw: &Value) -> Result<String, ProcessError> {
        raw.get("message")
            .and_then(|m| m.get("chat_id"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| {
                ProcessError::ProcessingFailed(
                    "SessionRouter: cannot extract 'to' (message.chat_id)".into(),
                )
            })
    }

    /// Extract channel name from the raw webhook.
    /// Falls back to "feishu" when no explicit channel field is present.
    fn extract_channel(raw: &Value) -> String {
        raw.get("channel")
            .and_then(|v| v.as_str())
            .unwrap_or("feishu")
            .to_string()
    }

    /// Extract account_id (tenant_key or app_id) from the raw webhook.
    fn extract_account_id(raw: &Value) -> Option<String> {
        raw.get("tenant_key")
            .or_else(|| raw.get("app_id"))
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    /// Extract thread_id from the raw webhook with priority:
    /// `message.thread_id` > `message.root_id` > `message.parent_id`.
    fn extract_thread_id(raw: &Value) -> Option<String> {
        raw.get("message").and_then(|m| {
            m.get("thread_id")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| m.get("root_id").and_then(|v| v.as_str()).map(String::from))
                .or_else(|| {
                    m.get("parent_id")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
        })
    }

    /// Resolve the original webhook JSON.
    ///
    /// When a RawLogProcessor runs upstream, `raw` becomes a serialized
    /// `ProcessedMessage` (only `content` + `metadata`). This method
    /// recovers the original webhook so routing fields can be read.
    fn get_webhook_raw(raw: &Value, ctx: &MessageContext) -> Value {
        // If raw already looks like an original webhook, use it.
        if raw.get("sender").is_some() || raw.get("message").is_some() {
            return raw.clone();
        }
        // Otherwise try ctx metadata (set by process_inbound).
        if let Some(wh) = ctx.metadata.get("_raw_webhook") {
            if let Ok(parsed) = serde_json::from_str::<Value>(wh) {
                return parsed;
            }
        }
        raw.clone()
    }

    /// Extract message content, supporting both webhook and
    /// ProcessedMessage layouts.
    fn extract_msg_content(webhook: &Value, raw: &Value) -> String {
        // Webhook layout: raw.message.content
        webhook
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|v| v.as_str())
            .map(String::from)
            // ProcessedMessage layout: raw.content
            .or_else(|| {
                raw.get("content")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .unwrap_or_default()
    }
}

impl SessionRouter {
    /// Compute a deterministic session key from routing fields,
    /// returning an empty string on missing fields.
    fn compute_key(
        &self,
        from: &str,
        to: &str,
        channel: &str,
        account_id: Option<&str>,
        thread_id: Option<String>,
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
            thread_id,
        };
        self.dm_scope.compute_session_key(channel, &msg, account_id)
    }
}

#[async_trait]
impl MessageProcessor for SessionRouter {
    fn priority(&self) -> i32 {
        20 // runs before FeishuMessageCleaner (30)
    }

    fn phase(&self) -> ProcessPhase {
        ProcessPhase::Inbound
    }

    async fn process(
        &self,
        ctx: &MessageContext,
        raw: &Value,
    ) -> Result<ProcessedMessage, ProcessError> {
        let webhook = Self::get_webhook_raw(raw, ctx);

        // Group chats are not supported.
        let is_group = webhook
            .get("message")
            .and_then(|m| m.get("chat_type"))
            .and_then(|v| v.as_str())
            .map(|ct| ct == "group")
            .unwrap_or(false);

        if is_group {
            let channel = Self::extract_channel(&webhook);
            return Err(ProcessError::SessionNotSupportedForChannel(channel));
        }

        let from = Self::extract_from(&webhook).unwrap_or_default();
        let to = Self::extract_to(&webhook).unwrap_or_default();
        let channel = Self::extract_channel(&webhook);
        let account_id = Self::extract_account_id(&webhook);
        let thread_id = Self::extract_thread_id(&webhook);

        let session_key = self.compute_key(&from, &to, &channel, account_id.as_deref(), thread_id);

        let mut metadata = ctx.metadata.clone();
        let acc_id = account_id.unwrap_or_else(|| "default".to_string());
        metadata.insert("account_id".to_string(), acc_id);
        metadata.insert("from".to_string(), from);
        metadata.insert("to".to_string(), to);
        metadata.insert("channel".to_string(), channel);
        metadata.insert("session_key".to_string(), session_key);

        let content = Self::extract_msg_content(&webhook, raw);

        Ok(ProcessedMessage { content, metadata })
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::DmScope;

    fn make_router() -> SessionRouter {
        SessionRouter::new(DmScope::PerAccountChannelPeer)
    }

    /// Minimal feishu DM webhook fixture.
    fn feishu_dm_webhook(from_open_id: &str, chat_id: &str) -> Value {
        serde_json::json!({
            "schema": "2.0",
            "event_type": "im.message.receive_v1",
            "tenant_key": "tenant_abc",
            "app_id": "app_xyz",
            "message": {
                "chat_id": chat_id,
                "chat_type": "p2p",
                "message_id": "om_123",
                "message_type": "text",
                "content": "{\"text\":\"hello\"}",
                "create_time": "1777229589621"
            },
            "sender": {
                "sender_id": { "open_id": from_open_id }
            }
        })
    }

    /// Group chat webhook fixture.
    fn feishu_group_webhook(from_open_id: &str, chat_id: &str) -> Value {
        serde_json::json!({
            "message": {
                "chat_id": chat_id,
                "chat_type": "group",
                "message_id": "om_123",
                "message_type": "text",
                "content": "{\"text\":\"hello\"}"
            },
            "sender": {
                "sender_id": { "open_id": from_open_id }
            }
        })
    }

    #[tokio::test]
    async fn test_private_chat_computes_session_key() {
        let router = make_router();
        let raw = feishu_dm_webhook("ou_user_a", "oc_agent_b");
        let result = router
            .process(&MessageContext::default(), &raw)
            .await
            .unwrap();
        let key = result.metadata.get("session_key").unwrap();
        assert!(!key.is_empty(), "session_key should not be empty");
        // PerAccountChannelPeer: acc:channel:from:to
        assert!(key.contains("ou_user_a"), "key should contain from: {key}");
        assert!(key.contains("oc_agent_b"), "key should contain to: {key}");
    }

    #[tokio::test]
    async fn test_deterministic_session_key() {
        let router = make_router();
        let raw = feishu_dm_webhook("ou_user_a", "oc_agent_b");
        let r1 = router
            .process(&MessageContext::default(), &raw)
            .await
            .unwrap();
        let key1 = r1.metadata.get("session_key").unwrap().clone();
        let r2 = router
            .process(&MessageContext::default(), &raw)
            .await
            .unwrap();
        let key2 = r2.metadata.get("session_key").unwrap().clone();
        assert_eq!(key1, key2, "same input must produce same session_key");
    }

    #[tokio::test]
    async fn test_group_chat_returns_error() {
        let router = make_router();
        let raw = feishu_group_webhook("ou_user_a", "oc_chat");
        let result = router.process(&MessageContext::default(), &raw).await;
        assert!(matches!(
            result,
            Err(ProcessError::SessionNotSupportedForChannel(_))
        ));
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("feishu") || msg.contains("oc_chat"), "{msg}");
    }

    #[tokio::test]
    async fn test_dm_scope_affects_session_key() {
        let r1 = SessionRouter::new(DmScope::PerChannelPeer);
        let r2 = SessionRouter::new(DmScope::PerAccountChannelPeer);
        let raw = feishu_dm_webhook("ou_user_a", "oc_agent_b");
        let k1 = r1
            .process(&MessageContext::default(), &raw)
            .await
            .unwrap()
            .metadata
            .get("session_key")
            .unwrap()
            .clone();
        let k2 = r2
            .process(&MessageContext::default(), &raw)
            .await
            .unwrap()
            .metadata
            .get("session_key")
            .unwrap()
            .clone();
        assert_ne!(k1, k2, "different DmScope should produce different keys");
    }

    #[tokio::test]
    async fn test_metadata_preserves_upstream_fields() {
        let router = make_router();
        let raw = feishu_dm_webhook("ou_user_a", "oc_agent_b");
        let mut ctx = MessageContext::default();
        ctx.metadata
            .insert("existing_key".to_string(), "existing_value".to_string());
        let result = router.process(&ctx, &raw).await.unwrap();
        assert_eq!(
            result.metadata.get("existing_key").unwrap(),
            "existing_value"
        );
        assert!(result.metadata.contains_key("session_key"));
        assert!(result.metadata.contains_key("from"));
        assert!(result.metadata.contains_key("to"));
        assert!(result.metadata.contains_key("channel"));
        assert!(result.metadata.contains_key("account_id"));
    }

    #[tokio::test]
    async fn test_processed_msg_fallback_from_ctx_raw_webhook() {
        let router = make_router();
        let webhook = feishu_dm_webhook("ou_user_a", "oc_agent_b");
        let mut ctx = MessageContext::default();
        ctx.metadata
            .insert("_raw_webhook".to_string(), webhook.to_string());
        let processed = serde_json::json!({
            "content": "{\"text\":\"hello\"}",
            "metadata": {}
        });
        let result = router.process(&ctx, &processed).await.unwrap();
        assert_eq!(result.metadata.get("from").unwrap(), "ou_user_a");
        assert_eq!(result.metadata.get("to").unwrap(), "oc_agent_b");
        assert_eq!(result.metadata.get("channel").unwrap(), "feishu");
        assert_eq!(result.metadata.get("account_id").unwrap(), "tenant_abc");
        let key = result.metadata.get("session_key").unwrap();
        assert!(!key.is_empty());
        assert!(key.contains("ou_user_a"));
        assert!(key.contains("oc_agent_b"));
        assert_eq!(result.content, "{\"text\":\"hello\"}");
    }

    /// ProcessedMessage input: routing fields from ctx.metadata["_raw_webhook"].
    #[tokio::test]
    async fn test_processed_msg_extracts_thread_id_from_raw_webhook() {
        let router = make_router();
        let webhook = serde_json::json!({
            "sender": {
                "sender_id": { "open_id": "ou_thread_user" }
            },
            "message": {
                "chat_id": "oc_thread_chat",
                "chat_type": "p2p",
                "message_id": "om_thread_001",
                "message_type": "text",
                "content": "{\"text\":\"original\"}",
                "create_time": "1777229589621",
                "thread_id": "ot_thread_xyz"
            },
            "channel": "feishu",
            "tenant_key": "tenant_thread"
        });
        let mut ctx = MessageContext::default();
        ctx.metadata
            .insert("_raw_webhook".to_string(), webhook.to_string());
        let processed = serde_json::json!({
            "content": "{\"text\":\"rewritten\"}",
            "metadata": {}
        });
        let result = router.process(&ctx, &processed).await.unwrap();
        assert_eq!(result.metadata.get("from").unwrap(), "ou_thread_user");
        assert_eq!(result.metadata.get("to").unwrap(), "oc_thread_chat");
        assert_eq!(result.metadata.get("channel").unwrap(), "feishu");
        assert_eq!(result.metadata.get("account_id").unwrap(), "tenant_thread");
        let key = result.metadata.get("session_key").unwrap();
        assert!(!key.is_empty());
        assert!(key.contains("ou_thread_user"));
        assert!(key.contains("oc_thread_chat"));
        assert_eq!(result.content, "{\"text\":\"original\"}");
    }

    #[tokio::test]
    async fn test_missing_fields_yields_empty_session_key() {
        let router = make_router();
        // A minimal payload with no sender/message fields.
        let raw = serde_json::json!({});
        let result = router
            .process(&MessageContext::default(), &raw)
            .await
            .unwrap();
        assert_eq!(
            result.metadata.get("session_key").unwrap(),
            "",
            "missing fields should yield empty session_key"
        );
    }
}
