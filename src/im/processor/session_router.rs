//! SessionRouter — inbound MessageProcessor that resolves session IDs.
//!
//! Extracts feishu routing fields (`account_id`, `from`, `to`, `channel`)
//! directly from the raw feishu webhook JSON, then calls
//! [`SessionManager::find_or_create`][crate::gateway::SessionManager::find_or_create]
//! and attaches the resulting `session_id` to the message metadata.
//!
//! Runs at priority 20, before [`FeishuMessageCleaner`] (priority 30).

use async_trait::async_trait;
use std::collections::BTreeMap;

use super::{MessageContext, MessageProcessor, ProcessError, ProcessPhase, ProcessedMessage};
use crate::gateway::{Message, SessionManager};
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
        // Group chats are not supported.
        let is_group = raw
            .get("message")
            .and_then(|m| m.get("chat_type"))
            .and_then(|v| v.as_str())
            .map(|ct| ct == "group")
            .unwrap_or(false);

        if is_group {
            let channel = Self::extract_channel(raw);
            return Err(ProcessError::SessionNotSupportedForChannel(channel));
        }

        // Extract feishu routing fields directly from the raw webhook.
        let from = Self::extract_from(raw)?;
        let to = Self::extract_to(raw)?;
        let channel = Self::extract_channel(raw);
        let account_id = Self::extract_account_id(raw);

        // Reconstruct a minimal Message for SessionManager::find_or_create.
        let msg_content = raw
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_default();

        let msg = Message {
            id: raw
                .get("message")
                .and_then(|m| m.get("message_id"))
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| "unknown".to_string()),
            from: from.clone(),
            to: to.clone(),
            content: msg_content.clone(),
            channel: channel.clone(),
            timestamp: raw
                .get("message")
                .and_then(|m| m.get("create_time"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or_else(|| chrono::Utc::now().timestamp()),
            metadata: std::collections::HashMap::new(),
        };

        // Resolve session.
        let session_id = self
            .manager
            .find_or_create(&channel, &msg, account_id.as_deref())
            .await
            .map_err(|e| ProcessError::ProcessingFailed(e.to_string()))?;

        // Preserve all upstream metadata (from ctx) and add our own fields.
        let mut metadata = ctx.metadata.clone();
        let acc_id = account_id.unwrap_or_else(|| "default".to_string());
        metadata.insert("account_id".to_string(), acc_id);
        metadata.insert("from".to_string(), from);
        metadata.insert("to".to_string(), to);
        metadata.insert("channel".to_string(), channel);
        metadata.insert("session_id".to_string(), session_id);

        // Pass the original raw webhook through so downstream FeishuMessageCleaner
        // (priority 30) can extract and clean the content.
        Ok(ProcessedMessage {
            content: msg_content,
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
    use std::sync::Arc;

    fn test_config() -> GatewayConfig {
        GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 65536,
            dm_scope: DmScope::PerAccountChannelPeer,
        }
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
        let mgr = Arc::new(SessionManager::new(&test_config(), None));
        let router = SessionRouter::new(mgr.clone());
        let raw = feishu_dm_webhook("ou_user_a", "oc_agent_b");

        let result = router
            .process(&MessageContext::default(), &raw)
            .await
            .unwrap();
        let session_id = result.metadata.get("session_id").unwrap();
        assert!(session_id.contains("feishu"), "{}", session_id);
        assert!(session_id.contains("ou_user_a"), "{}", session_id);
        assert!(session_id.contains("oc_agent_b"), "{}", session_id);
    }

    #[tokio::test]
    async fn test_existing_session_not_duplicated() {
        let mgr = Arc::new(SessionManager::new(&test_config(), None));
        let router = SessionRouter::new(mgr.clone());
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
        let mgr = Arc::new(SessionManager::new(&test_config(), None));
        let router = SessionRouter::new(mgr.clone());
        let raw = feishu_group_webhook("ou_user_a", "oc_chat");

        let result = router.process(&MessageContext::default(), &raw).await;
        assert!(matches!(
            result,
            Err(ProcessError::SessionNotSupportedForChannel(_))
        ));
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("feishu") || msg.contains("oc_chat"), "{}", msg);
    }

    #[tokio::test]
    async fn test_account_id_passed_to_find_or_create() {
        let cfg = GatewayConfig {
            dm_scope: DmScope::PerAccountChannelPeer,
            ..test_config()
        };
        let mgr = Arc::new(SessionManager::new(&cfg, None));
        let router = SessionRouter::new(mgr.clone());
        let raw = feishu_dm_webhook("ou_user_a", "oc_agent_b");

        let result = router
            .process(&MessageContext::default(), &raw)
            .await
            .unwrap();
        let session_id = result.metadata.get("session_id").unwrap();
        assert!(
            session_id.starts_with("tenant_abc:"),
            "session_id should include tenant_key prefix: {}",
            session_id
        );
    }

    #[tokio::test]
    async fn test_metadata_preserves_upstream_fields() {
        let mgr = Arc::new(SessionManager::new(&test_config(), None));
        let router = SessionRouter::new(mgr.clone());
        let raw = feishu_dm_webhook("ou_user_a", "oc_agent_b");

        let mut ctx = MessageContext::default();
        ctx.metadata
            .insert("existing_key".to_string(), "existing_value".to_string());

        let result = router.process(&ctx, &raw).await.unwrap();
        // Upstream metadata should be preserved.
        assert_eq!(
            result.metadata.get("existing_key").unwrap(),
            "existing_value"
        );
        // SessionRouter-added fields should be present.
        assert!(result.metadata.contains_key("session_id"));
        assert!(result.metadata.contains_key("from"));
        assert!(result.metadata.contains_key("to"));
        assert!(result.metadata.contains_key("channel"));
        assert!(result.metadata.contains_key("account_id"));
    }
}
