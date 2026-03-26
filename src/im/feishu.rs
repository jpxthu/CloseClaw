//! Feishu (Lark) IM Adapter
//!
//! Implements IMAdapter for Feishu messaging platform.

use super::{AdapterError, IMAdapter};
use crate::gateway::Message;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Duration;

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

/// Feishu API base URL
const FEISHU_API_BASE: &str = "https://open.feishu.cn/open-apis";

/// Feishu adapter implementation
#[derive(Debug, Clone)]
pub struct FeishuAdapter {
    app_id: String,
    app_secret: String,
    verification_token: String,
    http_client: Client,
}

impl FeishuAdapter {
    pub fn new(app_id: String, app_secret: String, verification_token: String) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("FeishuAdapter: failed to build HTTP client");
        Self {
            app_id,
            app_secret,
            verification_token,
            http_client,
        }
    }

    /// Obtain a tenant access token from Feishu API.
    async fn get_tenant_token(&self) -> Result<String, AdapterError> {
        #[derive(Serialize)]
        struct TokenRequest<'a> {
            app_id: &'a str,
            app_secret: &'a str,
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            code: i32,
            msg: String,
            tenant_access_token: Option<String>,
        }

        let resp: TokenResponse = self
            .http_client
            .post(format!(
                "{}/auth/v3/tenant_access_token/internal",
                FEISHU_API_BASE
            ))
            .json(&TokenRequest {
                app_id: &self.app_id,
                app_secret: &self.app_secret,
            })
            .send()
            .await
            .map_err(|e| AdapterError::SendFailed(e.to_string()))?
            .json()
            .await
            .map_err(|e| AdapterError::SendFailed(e.to_string()))?;

        if resp.code != 0 {
            return Err(AdapterError::SendFailed(format!(
                "Feishu token error {}: {}",
                resp.code, resp.msg
            )));
        }

        resp.tenant_access_token
            .ok_or_else(|| AdapterError::SendFailed("No token in response".to_string()))
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

        let text = content
            .get("text")
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
        let token = self.get_tenant_token().await?;

        #[derive(Serialize)]
        struct SendRequest<'a> {
            receive_id: &'a str,
            msg_type: &'a str,
            content: &'a str,
        }

        #[derive(Deserialize)]
        struct SendResponse {
            code: i32,
            msg: String,
        }

        let payload = SendRequest {
            receive_id: &message.to,
            msg_type: "text",
            content: &serde_json::json!({ "text": &message.content }).to_string(),
        };

        let resp: SendResponse = self
            .http_client
            .post(format!(
                "{}/im/v1/messages?receive_id_type=open_id",
                FEISHU_API_BASE
            ))
            .header("Authorization", format!("Bearer {}", token))
            .json(&payload)
            .send()
            .await
            .map_err(|e| AdapterError::SendFailed(e.to_string()))?
            .json()
            .await
            .map_err(|e| AdapterError::SendFailed(e.to_string()))?;

        if resp.code != 0 {
            return Err(AdapterError::SendFailed(format!(
                "Feishu send error {}: {}",
                resp.code, resp.msg
            )));
        }

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
