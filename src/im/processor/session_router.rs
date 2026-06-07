//! SessionRouter — inbound MessageProcessor that resolves session IDs.
//!
//! Extracts feishu routing fields (`account_id`, `from`, `to`, `channel`)
//! directly from the raw feishu webhook JSON, then calls
//! [`SessionManager::find_or_create`][crate::gateway::SessionManager::find_or_create]
//! and attaches the resulting `session_id` to the message metadata.
//!
//! Runs at priority 20, before [`FeishuMessageCleaner`] (priority 30).

use super::{MessageContext, MessageProcessor, ProcessError, ProcessPhase, ProcessedMessage};
use crate::gateway::{Message, SessionManager};
use async_trait::async_trait;
use serde_json::Value;

/// SessionRouter — resolves and attaches a session_id to the message pipeline.
#[derive(Debug, Clone)]
pub struct SessionRouter {
    manager: std::sync::Arc<SessionManager>,
}

impl SessionRouter {
    /// Create a new SessionRouter backed by the given SessionManager.
    pub fn new(manager: std::sync::Arc<SessionManager>) -> Self {
        Self { manager }
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

    /// Build a [`Message`] from webhook + extracted routing fields.
    fn build_message(
        webhook: &Value,
        raw: &Value,
        from: &str,
        to: &str,
        channel: &str,
        thread_id: Option<String>,
    ) -> Message {
        let content = Self::extract_msg_content(webhook, raw);
        let id = webhook
            .get("message")
            .and_then(|m| m.get("message_id"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| "unknown".to_string());
        let timestamp = webhook
            .get("message")
            .and_then(|m| m.get("create_time"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or_else(|| chrono::Utc::now().timestamp());
        Message {
            id,
            from: from.to_string(),
            to: to.to_string(),
            content,
            channel: channel.to_string(),
            timestamp,
            metadata: std::collections::HashMap::new(),
            thread_id,
        }
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

        let from = Self::extract_from(&webhook)?;
        let to = Self::extract_to(&webhook)?;
        let channel = Self::extract_channel(&webhook);
        let account_id = Self::extract_account_id(&webhook);
        let thread_id = Self::extract_thread_id(&webhook);

        let msg = Self::build_message(&webhook, raw, &from, &to, &channel, thread_id);

        let session_id = self
            .manager
            .find_or_create(&channel, &msg, account_id.as_deref())
            .await
            .map_err(|e| ProcessError::ProcessingFailed(e.to_string()))?;

        let mut metadata = ctx.metadata.clone();
        let acc_id = account_id.unwrap_or_else(|| "default".to_string());
        metadata.insert("account_id".to_string(), acc_id);
        metadata.insert("from".to_string(), from);
        metadata.insert("to".to_string(), to);
        metadata.insert("channel".to_string(), channel);
        metadata.insert("session_id".to_string(), session_id);

        Ok(ProcessedMessage {
            content: msg.content,
            metadata,
        })
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::{DmScope, GatewayConfig};
    use crate::session::bootstrap::BootstrapMode;
    use crate::session::persistence::ReasoningLevel;
    use std::sync::Arc;

    fn test_config() -> GatewayConfig {
        GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 65536,
            dm_scope: DmScope::PerAccountChannelPeer,
            ..Default::default()
        }
    }

    fn make_router() -> SessionRouter {
        let mgr = Arc::new(SessionManager::new(
            &test_config(),
            None,
            None,
            BootstrapMode::Full,
            ReasoningLevel::default(),
        ));
        SessionRouter::new(mgr)
    }

    fn assert_session_id_format(sid: &str, agent_id: &str) {
        assert!(
            sid.starts_with(&format!("{agent_id}_")),
            "bad format: {sid}"
        );
        let hex = sid.rsplitn(2, '_').next().unwrap();
        assert_eq!(hex.len(), 8, "hex part wrong: {hex}");
        assert!(
            hex.chars().all(|c| c.is_ascii_hexdigit()),
            "hex part non-hex: {hex}"
        );
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
    async fn test_private_chat_creates_session() {
        let router = make_router();
        let raw = feishu_dm_webhook("ou_user_a", "oc_agent_b");
        let result = router
            .process(&MessageContext::default(), &raw)
            .await
            .unwrap();
        let sid = result.metadata.get("session_id").unwrap();
        assert_session_id_format(sid, "oc_agent_b");
    }

    #[tokio::test]
    async fn test_existing_session_not_duplicated() {
        let router = make_router();
        let raw = feishu_dm_webhook("ou_user_a", "oc_agent_b");
        let r1 = router
            .process(&MessageContext::default(), &raw)
            .await
            .unwrap();
        let id1 = r1.metadata.get("session_id").unwrap().clone();
        let r2 = router
            .process(&MessageContext::default(), &raw)
            .await
            .unwrap();
        let id2 = r2.metadata.get("session_id").unwrap().clone();
        assert_eq!(id1, id2, "session should not be duplicated");
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
    async fn test_account_id_passed_to_find_or_create() {
        let cfg = GatewayConfig {
            dm_scope: DmScope::PerAccountChannelPeer,
            ..test_config()
        };
        let mgr = Arc::new(SessionManager::new(
            &cfg,
            None,
            None,
            BootstrapMode::Full,
            ReasoningLevel::default(),
        ));
        let router = SessionRouter::new(mgr);
        let raw = feishu_dm_webhook("ou_user_a", "oc_agent_b");
        let result = router
            .process(&MessageContext::default(), &raw)
            .await
            .unwrap();
        let sid = result.metadata.get("session_id").unwrap();
        assert_session_id_format(sid, "oc_agent_b");
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
        assert!(result.metadata.contains_key("session_id"));
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
        assert_session_id_format(result.metadata.get("session_id").unwrap(), "oc_agent_b");
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
        assert!(result
            .metadata
            .get("session_id")
            .unwrap()
            .starts_with("oc_thread_chat_"));
        assert_eq!(result.content, "{\"text\":\"original\"}");
    }

    #[tokio::test]
    async fn test_processed_msg_without_raw_webhook_errors() {
        let router = make_router();
        let processed = serde_json::json!({
            "content": "{\"text\":\"hi\"}",
            "metadata": {}
        });
        let ctx = MessageContext::default();
        let err = router.process(&ctx, &processed).await;
        assert!(err.is_err(), "should fail without raw webhook");
    }
}
