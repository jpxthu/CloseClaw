//! Feishu (Lark) IM Adapter
//!
//! Implements IMAdapter for Feishu messaging platform.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use sha2::{Sha256, Digest};
use crate::gateway::Message;
use super::{IMAdapter, AdapterError};

/// Feishu webhook event payload
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FeishuEvent {
    schema: String,
    header: FeishuHeader,
    event: FeishuMessageEvent,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FeishuHeader {
    event_id: String,
    event_type: String,
    create_time: String,
    token: String,
    app_id: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FeishuMessageEvent {
    sender: FeishuSender,
    content: String,
    chat_id: String,
    message_type: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FeishuSender {
    sender_id: FeishuSenderId,
    sender_type: String,
}

#[derive(Debug, Deserialize)]
struct FeishuSenderId {
    open_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct FeishuOutgoingPayload {
    open_id: String,
    msg_type: String,
    content: String,
}

/// Feishu adapter implementation
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FeishuAdapter {
    app_id: String,
    app_secret: String,
    verification_token: String,
}

impl FeishuAdapter {
    pub fn new(app_id: String, app_secret: String, verification_token: String) -> Self {
        Self {
            app_id,
            app_secret,
            verification_token,
        }
    }
}

#[async_trait]
impl IMAdapter for FeishuAdapter {
    fn name(&self) -> &str {
        "feishu"
    }

    async fn handle_webhook(&self, payload: &[u8]) -> Result<Message, AdapterError> {
        let event: FeishuEvent = serde_json::from_slice(payload)
            .map_err(|e| AdapterError::InvalidPayload(e.to_string()))?;

        let content: serde_json::Value = serde_json::from_str(&event.event.content)
            .map_err(|e| AdapterError::InvalidPayload(e.to_string()))?;

        let text = content.get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();

        Ok(Message {
            id: event.header.event_id,
            from: event.event.sender.sender_id.open_id,
            to: String::new(), // Will be filled by gateway
            content: text,
            channel: "feishu".to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            metadata: HashMap::new(),
        })
    }

    async fn send_message(&self, message: &Message) -> Result<(), AdapterError> {
        let _payload = FeishuOutgoingPayload {
            open_id: message.to.clone(),
            msg_type: "text".to_string(),
            content: serde_json::json!({ "text": message.content }).to_string(),
        };

        // In production, this would call Feishu API
        // For now, just return Ok
        Ok(())
    }

    async fn validate_signature(&self, signature: &str, payload: &[u8]) -> bool {
        let mut hasher = Sha256::new();
        hasher.update(&self.verification_token);
        hasher.update(payload);
        let result = hasher.finalize();
        let expected = format!("{:x}", result);
        expected == signature
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feishu_adapter_name() {
        let adapter = FeishuAdapter::new(
            "app_id".to_string(),
            "app_secret".to_string(),
            "token".to_string(),
        );
        assert_eq!(adapter.name(), "feishu");
    }
}
