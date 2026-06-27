//! Feishu adapter — HTTP I/O, token management, and webhook parsing.

use crate::error::AdapterError;
use crate::normalized::NormalizedMessage;
use crate::IMAdapter;
use async_trait::async_trait;
use closeclaw_gateway::Message;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Webhook event types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(super) struct FeishuEvent {
    pub(super) schema: String,
    pub(super) header: FeishuHeader,
    pub(super) event: FeishuMessageEvent,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(super) struct FeishuHeader {
    pub(super) event_id: String,
    pub(super) event_type: String,
    pub(super) create_time: String,
    pub(super) token: String,
    pub(super) app_id: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(super) struct FeishuMessageEvent {
    pub(super) sender: FeishuSender,
    pub(super) content: String,
    pub(super) chat_id: String,
    pub(super) message_type: String,
    #[serde(default)]
    pub(super) thread_id: Option<String>,
    #[serde(default)]
    pub(super) root_id: Option<String>,
    #[serde(default)]
    pub(super) parent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(super) struct FeishuSender {
    pub(super) sender_id: FeishuSenderId,
    pub(super) sender_type: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct FeishuSenderId {
    pub(super) open_id: String,
}

/// Card action event payload (`card.action.trigger`).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(super) struct FeishuCardActionEvent {
    pub(super) operator: FeishuCardOperator,
    pub(super) token: String,
    pub(super) action: FeishuCardAction,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(super) struct FeishuCardOperator {
    pub(super) open_id: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(super) struct FeishuCardAction {
    pub(super) value: Option<serde_json::Value>,
    pub(super) tag: Option<String>,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const FEISHU_API_BASE: &str = "https://open.feishu.cn/open-apis";

// ---------------------------------------------------------------------------
// Post content expansion
// ---------------------------------------------------------------------------

#[allow(dead_code)]
/// Expand a Feishu post-type content JSON value into plain text.
///
/// The `content` parameter is the parsed JSON object with `title` (optional)
/// and `content` (2D array of elements, each element has a `tag` field).
///
/// - `title` becomes the first line (if present).
/// - Each sub-array in `content` becomes one line; elements are concatenated.
/// - Supported tags: `text`, `a`, `at`, unknown tags use `text` if available.
fn expand_post_content(content: &serde_json::Value) -> String {
    let mut lines: Vec<String> = Vec::new();

    // Extract title as the first line if present.
    if let Some(title) = content.get("title").and_then(|t| t.as_str()) {
        if !title.is_empty() {
            lines.push(title.to_string());
        }
    }

    // Iterate over the 2D content array.
    if let Some(rows) = content.get("content").and_then(|c| c.as_array()) {
        for row in rows {
            let row_text: String = row
                .as_array()
                .map(|elements| {
                    elements
                        .iter()
                        .map(expand_element)
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default();
            lines.push(row_text);
        }
    }

    lines.join("\n")
}

#[allow(dead_code)]
/// Expand a single post content element into plain text based on its tag.
///
/// Supported tags:
/// - `text`, `a` → text content
/// - `at` → `@name` or `@user_id`
/// - `img` → `[图片]`
/// - `media` → `[视频]`
/// - `file` → `[文件]`
/// - unknown tags → text if available, otherwise `[未知消息]`
fn expand_element(elem: &serde_json::Value) -> String {
    let tag = elem.get("tag").and_then(|t| t.as_str()).unwrap_or("");
    match tag {
        "text" | "a" => elem
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        "at" => {
            if let Some(name) = elem.get("name").and_then(|n| n.as_str()) {
                format!("@{}", name)
            } else if let Some(user_id) = elem.get("user_id").and_then(|u| u.as_str()) {
                format!("@{}", user_id)
            } else {
                elem.get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string()
            }
        }
        "img" => "[图片]".to_string(),
        "media" => "[视频]".to_string(),
        "file" => "[文件]".to_string(),
        _ => {
            let text = elem.get("text").and_then(|t| t.as_str()).unwrap_or("");
            if text.is_empty() {
                "[未知消息]".to_string()
            } else {
                text.to_string()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// CachedToken
// ---------------------------------------------------------------------------

/// Cached tenant access token with expiry time.
#[derive(Debug, Clone)]
pub struct CachedToken {
    pub token: String,
    pub expires_at: Instant,
}

impl CachedToken {
    /// Returns true if token is expired or close to expiry (within 5 minutes).
    pub fn needs_refresh(&self) -> bool {
        Instant::now() > self.expires_at - Duration::from_secs(300)
    }
}

// ---------------------------------------------------------------------------
// FeishuAdapter
// ---------------------------------------------------------------------------

/// Feishu adapter implementation.
#[derive(Debug, Clone)]
pub struct FeishuAdapter {
    app_id: String,
    app_secret: String,
    verification_token: String,
    http_client: Client,
    pub(super) cached_token: Arc<Mutex<Option<CachedToken>>>,
    base_url: String,
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
            cached_token: Arc::new(Mutex::new(None)),
            base_url: FEISHU_API_BASE.to_string(),
        }
    }

    /// Obtain a tenant access token, using a cached token when valid.
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
                self.base_url
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
            .patch(format!("{}/im/v1/messages/{}", self.base_url, message_id))
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

    /// Handle a card.action.trigger event.
    pub(super) fn handle_card_action(
        &self,
        _event_id: String,
        _app_id: String,
        card_event: &FeishuCardActionEvent,
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        let action_value = card_event
            .action
            .value
            .as_ref()
            .and_then(|v| v.get("action"))
            .and_then(|a| a.as_str());

        match action_value {
            Some("forceful_shutdown") => {
                let mut metadata = HashMap::from([
                    (
                        "account_id".to_string(),
                        card_event.operator.open_id.clone(),
                    ),
                    ("card_action".to_string(), "true".to_string()),
                ]);
                if let Some(chat_id) = card_event
                    .action
                    .value
                    .as_ref()
                    .and_then(|v| v.get("chat_id"))
                    .and_then(|c| c.as_str())
                {
                    metadata.insert("chat_id".to_string(), chat_id.to_string());
                }
                Ok(Some(NormalizedMessage {
                    platform: "feishu".to_string(),
                    sender_id: card_event.operator.open_id.clone(),
                    peer_id: metadata.get("chat_id").cloned().unwrap_or_default(),
                    content: "/__card_action:forceful_shutdown".to_string(),
                    timestamp: chrono::Utc::now().timestamp_millis(),
                    message_type: "text".to_string(),
                    media_refs: vec![],
                    quoted_message: None,
                    thread_id: None,
                    account_id: metadata.get("account_id").cloned(),
                    card_action: Some(true),
                }))
            }
            _ => Ok(None),
        }
    }

    /// Parse a regular message event into a Message.
    ///
    /// Returns `Ok(None)` for non-text message types (image, file, audio, etc.).
    fn parse_message_event(
        &self,
        event: FeishuEvent,
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        let content: serde_json::Value = serde_json::from_str(&event.event.content)
            .map_err(|e| AdapterError::InvalidPayload(e.to_string()))?;

        let (text, message_type) = match event.event.message_type.as_str() {
            "text" => (
                content
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string(),
                "text".to_string(),
            ),
            "post" => (expand_post_content(&content), "post".to_string()),
            _other => return Ok(None),
        };

        let thread_id = event
            .event
            .thread_id
            .or(event.event.root_id)
            .or(event.event.parent_id);

        let sender_open_id = event.event.sender.sender_id.open_id;

        Ok(Some(NormalizedMessage {
            platform: "feishu".to_string(),
            sender_id: sender_open_id.clone(),
            peer_id: event.event.chat_id,
            content: text,
            timestamp: chrono::Utc::now().timestamp_millis(),
            message_type,
            media_refs: vec![],
            quoted_message: None,
            thread_id,
            account_id: Some(sender_open_id),
            card_action: None,
        }))
    }
}

#[async_trait]
impl IMAdapter for FeishuAdapter {
    fn name(&self) -> &str {
        "feishu"
    }

    async fn handle_webhook(
        &self,
        payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        let raw: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| AdapterError::InvalidPayload(e.to_string()))?;

        let event_type = raw
            .get("header")
            .and_then(|h| h.get("event_type"))
            .and_then(|t| t.as_str())
            .unwrap_or("");

        let event_id = raw
            .get("header")
            .and_then(|h| h.get("event_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let app_id = raw
            .get("header")
            .and_then(|h| h.get("app_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        match event_type {
            "card.action.trigger" => {
                let card_event: FeishuCardActionEvent = serde_json::from_value(raw)
                    .map_err(|e| AdapterError::InvalidPayload(e.to_string()))?;
                self.handle_card_action(event_id, app_id, &card_event)
            }
            _ => {
                let event: FeishuEvent = serde_json::from_value(raw)
                    .map_err(|e| AdapterError::InvalidPayload(e.to_string()))?;
                self.parse_message_event(event)
            }
        }
    }

    async fn send_message(
        &self,
        message: &Message,
        root_id: Option<&str>,
    ) -> Result<(), AdapterError> {
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

        let mut url = format!("{}/im/v1/messages?receive_id_type=chat_id", self.base_url);
        if let Some(rid) = root_id {
            let encoded_rid: String =
                url::form_urlencoded::byte_serialize(rid.as_bytes()).collect();
            url = format!("{}&root_id={}", url, encoded_rid);
        }

        let resp: SendResponse = self
            .http_client
            .post(&url)
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

    async fn send_card_json(
        &self,
        chat_id: &str,
        card_json: &str,
        root_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        let token = self.get_tenant_token().await?;

        #[derive(Serialize)]
        struct CardRequest<'a> {
            receive_id: &'a str,
            msg_type: &'a str,
            content: &'a str,
        }

        #[derive(Deserialize)]
        struct CardResponse {
            code: i32,
            msg: String,
        }

        let payload = CardRequest {
            receive_id: chat_id,
            msg_type: "interactive",
            content: card_json,
        };

        let mut url = format!("{}/im/v1/messages?receive_id_type=chat_id", self.base_url);
        if let Some(rid) = root_id {
            let encoded_rid: String =
                url::form_urlencoded::byte_serialize(rid.as_bytes()).collect();
            url = format!("{}&root_id={}", url, encoded_rid);
        }

        let resp: CardResponse = self
            .http_client
            .post(&url)
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
                "Feishu card send error {}: {}",
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
mod adapter_tests;
