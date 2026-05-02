//! Feishu (Lark) IM Adapter
//!
//! Implements IMAdapter for Feishu messaging platform.

use super::{AdapterError, IMAdapter};
use crate::card::{render_feishu_card, RichCard};
use crate::gateway::Message;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

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

/// Cached tenant access token with expiry time.
/// Feishu tokens are valid ~2 hours; we refresh proactively at 1.5h.
#[derive(Debug, Clone)]
pub struct CachedToken {
    pub token: String,
    /// When this token expires (absolute time)
    pub expires_at: Instant,
}

impl CachedToken {
    /// Returns true if token is expired or close to expiry (within 5 minutes)
    pub fn needs_refresh(&self) -> bool {
        Instant::now() > self.expires_at - Duration::from_secs(300)
    }
}

/// Feishu adapter implementation
#[derive(Debug, Clone)]
pub struct FeishuAdapter {
    app_id: String,
    app_secret: String,
    verification_token: String,
    http_client: Client,
    /// Cached tenant access token — shared across all clones via Arc<Mutex>
    cached_token: Arc<Mutex<Option<CachedToken>>>,
}

/// Constructor and token management
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
            cached_token: Arc::new(Mutex::new(None)),
        }
    }

    /// Obtain a tenant access token, using a cached token when valid.
    /// Feishu tokens are valid ~2 hours; we proactively refresh at 1.5h.
    async fn get_tenant_token(&self) -> Result<String, AdapterError> {
        let cached = self.cached_token.lock().await;
        if let Some(ref c) = *cached {
            if !c.needs_refresh() {
                return Ok(c.token.clone());
            }
        }
        drop(cached);

        let new_token = self.fetch_tenant_token().await?;

        let mut cached = self.cached_token.lock().await;
        *cached = Some(CachedToken {
            expires_at: Instant::now() + Duration::from_secs(7200),
            token: new_token.clone(),
        });

        Ok(new_token)
    }

    /// Fetch a fresh tenant access token from Feishu API (no caching).
    pub async fn fetch_tenant_token(&self) -> Result<String, AdapterError> {
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

/// Card send and update operations
impl FeishuAdapter {
    /// Build the JSON content string for a card send request.
    pub fn build_card_body(card: &RichCard) -> Result<String, AdapterError> {
        let payload = render_feishu_card(card);
        serde_json::to_string(&payload).map_err(|e| AdapterError::SendFailed(e.to_string()))
    }

    /// Send an interactive card message to a chat.
    /// Returns the message ID on success, needed for subsequent updates.
    pub async fn send_card(&self, chat_id: &str, card: &RichCard) -> Result<String, AdapterError> {
        let token = self.get_tenant_token().await?;
        let content = Self::build_card_body(card)?;
        let url = format!("{}/im/v1/messages?receive_id_type=open_id", FEISHU_API_BASE);

        #[derive(Serialize)]
        struct Req<'a> {
            receive_id: &'a str,
            msg_type: &'a str,
            content: &'a str,
        }
        #[derive(Deserialize)]
        struct Resp {
            code: i32,
            msg: String,
            data: Option<RespData>,
        }
        #[derive(Deserialize)]
        struct RespData {
            message_id: Option<String>,
        }

        let resp: Resp = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&Req {
                receive_id: chat_id,
                msg_type: "interactive",
                content: &content,
            })
            .send()
            .await
            .map_err(|e| AdapterError::SendFailed(e.to_string()))?
            .json()
            .await
            .map_err(|e| AdapterError::SendFailed(e.to_string()))?;

        if resp.code != 0 {
            return Err(AdapterError::SendFailed(format!(
                "Feishu card send error {}: {}",
                resp.code, resp.msg
            )));
        }
        resp.data.and_then(|d| d.message_id).ok_or_else(|| {
            AdapterError::SendFailed("No message_id in card send response".to_string())
        })
    }
}

impl FeishuAdapter {
    /// Update an existing card message identified by `message_id`.
    pub async fn update_message(
        &self,
        message_id: &str,
        patch: &serde_json::Value,
    ) -> Result<(), AdapterError> {
        let token = self.get_tenant_token().await?;

        #[derive(Serialize)]
        struct UpdateRequest<'a> {
            content: &'a str,
        }

        #[derive(Deserialize)]
        struct UpdateResponse {
            code: i32,
            msg: String,
        }

        let content =
            serde_json::to_string(patch).map_err(|e| AdapterError::SendFailed(e.to_string()))?;

        let resp: UpdateResponse = self
            .http_client
            .patch(format!("{}/im/v1/messages/{}", FEISHU_API_BASE, message_id))
            .header("Authorization", format!("Bearer {}", token))
            .json(&UpdateRequest { content: &content })
            .send()
            .await
            .map_err(|e| AdapterError::SendFailed(e.to_string()))?
            .json()
            .await
            .map_err(|e| AdapterError::SendFailed(e.to_string()))?;

        if resp.code != 0 {
            return Err(AdapterError::SendFailed(format!(
                "Feishu card update error {}: {}",
                resp.code, resp.msg
            )));
        }

        Ok(())
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
            metadata: HashMap::from([("account_id".to_string(), event.header.app_id.clone())]),
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
