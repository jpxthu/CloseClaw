//! Feishu adapter — HTTP I/O, token management, and webhook parsing.

use crate::error::AdapterError;
use crate::IMAdapter;
use async_trait::async_trait;
use closeclaw_common::{CardActionEvent, MediaRef, MediaType, MessageType, NormalizedMessage};
use closeclaw_gateway::Message;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::post_expand::expand_post_content;
use tokio::sync::Mutex;

// Webhook event types

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct FeishuEvent {
    pub(crate) schema: String,
    pub(crate) header: FeishuHeader,
    pub(crate) event: FeishuMessageEvent,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct FeishuHeader {
    pub(crate) event_id: String,
    pub(crate) event_type: String,
    pub(crate) create_time: String,
    pub(crate) token: String,
    pub(crate) app_id: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct FeishuMessageEvent {
    #[serde(default)]
    pub(crate) message_id: Option<String>,
    pub(crate) sender: FeishuSender,
    pub(crate) content: String,
    pub(crate) chat_id: String,
    pub(crate) message_type: String,
    #[serde(default)]
    pub(crate) thread_id: Option<String>,
    #[serde(default)]
    pub(crate) root_id: Option<String>,
    #[serde(default)]
    pub(crate) parent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct FeishuSender {
    pub(crate) sender_id: FeishuSenderId,
    pub(crate) sender_type: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FeishuSenderId {
    pub(crate) open_id: String,
}

/// Card action event payload (`card.action.trigger`).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct FeishuCardActionEvent {
    pub(crate) operator: FeishuCardOperator,
    pub(crate) token: String,
    pub(crate) action: FeishuCardAction,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct FeishuCardOperator {
    pub(crate) open_id: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct FeishuCardAction {
    pub(crate) value: Option<serde_json::Value>,
    pub(crate) tag: Option<String>,
}

pub(crate) const FEISHU_API_BASE: &str = "https://open.feishu.cn/open-apis";

/// Returns `true` when the Feishu API error code indicates a platform
/// capability limitation (e.g. unsupported `select_static` component).
///
/// These errors warrant a one-time fallback retry via text message.
/// Network failures, token errors, and permission errors are NOT
/// capability errors.
///
/// Error code sources (Feishu Open Platform documentation):
/// - 230001: invalid card element type
/// - 230002: unsupported component in card template
pub(crate) fn is_capability_error(code: i32) -> bool {
    matches!(code, 230001 | 230002)
}

// ---------------------------------------------------------------------------
// Quote helpers
// ---------------------------------------------------------------------------

/// Truncate text to at most 500 characters, appending "..." if truncated.
pub(crate) fn truncate_to_500(text: &str) -> String {
    let max_chars = 500;
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let byte_index = text
            .char_indices()
            .nth(max_chars)
            .map_or(text.len(), |(i, _)| i);
        format!("{}...", &text[..byte_index])
    }
}

/// Format text as a markdown blockquote: each line prefixed with "> ".
pub(crate) fn to_blockquote(text: &str) -> String {
    text.lines()
        .map(|line| format!("> {}", line))
        .collect::<Vec<_>>()
        .join("\n")
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
// Feishu API response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct FeishuMsgBody {
    content: Option<String>,
}

#[derive(Deserialize)]
struct FeishuMsgItem {
    msg_type: Option<String>,
    body: Option<FeishuMsgBody>,
}

#[derive(Deserialize)]
struct FeishuGetMessageResponse {
    code: i32,
    msg: String,
    items: Option<Vec<FeishuMsgItem>>,
}

#[derive(Deserialize)]
pub(crate) struct SendResponse {
    pub(crate) code: i32,
    pub(crate) msg: String,
}

// ---------------------------------------------------------------------------
// FeishuAdapter
// ---------------------------------------------------------------------------

/// Feishu adapter implementation.
#[derive(Debug, Clone)]
pub struct FeishuAdapter {
    pub(crate) app_id: String,
    pub(crate) app_secret: String,
    pub(crate) verification_token: String,
    pub(crate) http_client: Client,
    pub(crate) cached_token: Arc<Mutex<Option<CachedToken>>>,
    pub(crate) base_url: String,
    /// Metadata produced by the last successful `parse_inbound` call.
    /// Used by `last_parsed_metadata()` to surface platform-specific
    /// fields (e.g. `chat_name`) that were removed from NormalizedMessage.
    pub(crate) last_metadata: Arc<Mutex<HashMap<String, String>>>,
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
            last_metadata: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Extract the event_type from a raw webhook JSON payload header.
    fn extract_event_type(raw: &serde_json::Value) -> String {
        raw.get("header")
            .and_then(|h| h.get("event_type"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string()
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

    /// Fetch the content of a message by its ID via Feishu API.
    ///
    /// Returns `Some(text)` for supported types (text, post), or `None` for
    /// unsupported types or on failure (which logs a warning and degrades
    /// gracefully).
    pub async fn fetch_message_content(
        &self,
        message_id: &str,
    ) -> Result<Option<String>, AdapterError> {
        let (msg_type, raw_content) = match self.fetch_message_raw(message_id).await? {
            Some(pair) => pair,
            None => return Ok(None),
        };
        self.extract_text_from_message(&msg_type, &raw_content, message_id)
    }

    /// Fetch the raw message item from Feishu API and return (msg_type, content).
    async fn fetch_message_raw(
        &self,
        message_id: &str,
    ) -> Result<Option<(String, String)>, AdapterError> {
        let token = self.get_tenant_token().await?;
        let resp: FeishuGetMessageResponse = self
            .http_client
            .get(format!("{}/im/v1/messages/{}", self.base_url, message_id))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| AdapterError::SendFailed(e.to_string()))?
            .json()
            .await
            .map_err(|e| AdapterError::SendFailed(e.to_string()))?;

        if resp.code != 0 {
            tracing::warn!(
                code = resp.code,
                msg = %resp.msg,
                message_id = %message_id,
                "Failed to fetch quoted message"
            );
            return Ok(None);
        }

        let item = Self::extract_first_item(resp.items, message_id)?;
        let msg_type = item.msg_type.unwrap_or_default();
        let raw_content = match item.body.and_then(|b| b.content) {
            Some(c) => c,
            None => {
                tracing::warn!(
                    message_id = %message_id,
                    "Message body has no content"
                );
                return Ok(None);
            }
        };
        Ok(Some((msg_type, raw_content)))
    }

    /// Extract the first item from message response items, logging warnings.
    fn extract_first_item(
        items: Option<Vec<FeishuMsgItem>>,
        message_id: &str,
    ) -> Result<FeishuMsgItem, AdapterError> {
        let items = match items {
            Some(v) => v,
            None => {
                tracing::warn!(
                    message_id = %message_id,
                    "No items in message response"
                );
                return Err(AdapterError::SendFailed(
                    "No items in message response".to_string(),
                ));
            }
        };
        items.into_iter().next().ok_or_else(|| {
            tracing::warn!(message_id = %message_id, "Empty items in message response");
            AdapterError::SendFailed("Empty items in message response".to_string())
        })
    }

    /// Fetch the chat (group) name for a given chat_id via Feishu API.
    ///
    /// Returns `Some(name)` on success, or `None` on failure (which logs
    /// a warning and degrades gracefully — chat_name defaults to empty).
    pub async fn fetch_chat_name(&self, chat_id: &str) -> Option<String> {
        #[derive(Deserialize)]
        struct FeishuChatResponse {
            code: i32,
            msg: String,
            data: Option<FeishuChatData>,
        }
        #[derive(Deserialize)]
        struct FeishuChatData {
            name: Option<String>,
        }

        let token = self.get_tenant_token().await.ok()?;
        let resp: FeishuChatResponse = self
            .http_client
            .get(format!("{}/im/v1/chats/{}", self.base_url, chat_id))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;

        if resp.code != 0 {
            tracing::warn!(
                code = resp.code, msg = %resp.msg, chat_id = %chat_id,
                "Failed to fetch chat info"
            );
            return None;
        }
        resp.data.and_then(|d| d.name)
    }

    /// Extract readable text from a message's raw content based on msg_type.
    fn extract_text_from_message(
        &self,
        msg_type: &str,
        raw_content: &str,
        message_id: &str,
    ) -> Result<Option<String>, AdapterError> {
        match msg_type {
            "text" => {
                let parsed: serde_json::Value =
                    serde_json::from_str(raw_content).unwrap_or(serde_json::Value::Null);
                Ok(parsed
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(String::from))
            }
            "post" => {
                let parsed: serde_json::Value =
                    serde_json::from_str(raw_content).unwrap_or(serde_json::Value::Null);
                let text = expand_post_content(&parsed);
                if text.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(text))
                }
            }
            other => {
                tracing::debug!(
                    msg_type = other,
                    message_id = %message_id,
                    "Unsupported message type for quote"
                );
                Ok(None)
            }
        }
    }

    /// Build the URL for fetching a media resource, percent-encoding path params.
    fn build_media_resource_url(
        &self,
        message_id: &str,
        file_key: &str,
        resource_type: &str,
    ) -> String {
        let enc_msg: String = url::form_urlencoded::byte_serialize(message_id.as_bytes()).collect();
        let enc_key: String = url::form_urlencoded::byte_serialize(file_key.as_bytes()).collect();
        format!(
            "{}/im/v1/messages/{}/resources/{}?type={}",
            self.base_url, enc_msg, enc_key, resource_type
        )
    }

    /// Fetch a temporary download URL for a media resource (image, file, audio).
    pub(crate) async fn fetch_media_download_url(
        &self,
        message_id: &str,
        file_key: &str,
        resource_type: &str,
    ) -> Result<String, AdapterError> {
        let token = self.get_tenant_token().await?;
        #[derive(Deserialize)]
        struct ResourceResp {
            code: i32,
            msg: String,
            data: Option<serde_json::Value>,
        }
        let resp: ResourceResp = self
            .http_client
            .get(self.build_media_resource_url(message_id, file_key, resource_type))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| AdapterError::SendFailed(e.to_string()))?
            .json()
            .await
            .map_err(|e| AdapterError::SendFailed(e.to_string()))?;
        if resp.code != 0 {
            tracing::warn!(
                code = resp.code, msg = %resp.msg,
                message_id = %message_id, file_key = %file_key,
                "Failed to fetch media download URL"
            );
            return Err(AdapterError::SendFailed(format!(
                "Feishu media resource error {}: {}",
                resp.code, resp.msg
            )));
        }
        let url = resp
            .data
            .and_then(|d| d.get("url").and_then(|u| u.as_str()).map(String::from))
            .filter(|u| !u.is_empty())
            .ok_or_else(|| {
                tracing::warn!(message_id = %message_id, file_key = %file_key,
                    "No download URL in media resource response");
                AdapterError::SendFailed("No download URL in media resource response".to_string())
            })?;
        Ok(url)
    }

    /// Fetch and prepend a markdown blockquote for the quoted message.
    async fn prepend_quote_blockquote(&self, parent_id: Option<&str>, text: &str) -> String {
        let pid = match parent_id {
            Some(p) => p,
            None => return text.to_string(),
        };
        match self.fetch_message_content(pid).await {
            Ok(Some(quoted)) => {
                let truncated = truncate_to_500(&quoted);
                let blockquote = to_blockquote(&truncated);
                format!("{}\n\n{}", blockquote, text)
            }
            Ok(None) => text.to_string(),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    parent_id = %pid,
                    "Failed to fetch quoted message, proceeding without quote"
                );
                text.to_string()
            }
        }
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

    /// Parse a card.action.trigger event into a CardActionEvent.
    pub(crate) fn parse_card_action_event(
        &self,
        _event_id: String,
        _app_id: String,
        card_event: &FeishuCardActionEvent,
    ) -> Result<Option<CardActionEvent>, AdapterError> {
        let action_value = card_event
            .action
            .value
            .as_ref()
            .and_then(|v| v.get("action"))
            .and_then(|a| a.as_str());

        match action_value {
            Some(action) => {
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
                Ok(Some(CardActionEvent {
                    platform: "feishu".to_string(),
                    sender_id: card_event.operator.open_id.clone(),
                    action_value: action.to_string(),
                    metadata,
                    timestamp: chrono::Utc::now().timestamp_millis(),
                    account_id: card_event.operator.open_id.clone(),
                }))
            }
            _ => Ok(None),
        }
    }

    /// Parse a regular message event into a NormalizedMessage.
    /// For non-text messages, produces a message with `media_refs` populated.
    pub(crate) async fn parse_message_event(
        &self,
        event: FeishuEvent,
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        let content: serde_json::Value = serde_json::from_str(&event.event.content)
            .map_err(|e| AdapterError::InvalidPayload(e.to_string()))?;

        let original_parent_id = event.event.parent_id.clone();
        let thread_id = event
            .event
            .thread_id
            .or(event.event.root_id)
            .or(event.event.parent_id);
        let sender_open_id = event.event.sender.sender_id.open_id.clone();

        let (text, media_refs) =
            match Self::extract_message_content(&event.event.message_type, &content) {
                Ok(pair) => pair,
                Err(_) => return Ok(None),
            };

        // Populate media download URLs (non-text messages).
        // After Step 1.1 the MediaRef uses `path` (local) instead of `url`
        // (remote). For now the adapter does not perform local persistence,
        // so we leave `path` empty. The platform key (`r.key`) is already
        // set by `make_media_ref`.
        let mut media_refs = media_refs;
        for r in &mut media_refs {
            let msg_id = event.event.message_id.as_deref().unwrap_or("");
            let _url = self
                .fetch_media_download_url(msg_id, &r.key, &event.event.message_type)
                .await
                .unwrap_or_default();
            // TODO(media-store): download to local path, set r.path, r.size, r.mime
        }

        // Discard empty text content (only for text/post;
        // non-text messages have empty content by design).
        let is_text_type = matches!(
            event.event.message_type.as_str(),
            "text" | "post" | "sticker"
        );
        if is_text_type && text.trim().is_empty() {
            tracing::debug!(
                message_type = %event.event.message_type,
                "Discarding empty text content"
            );
            return Ok(None);
        }

        let content = self
            .prepend_quote_blockquote(original_parent_id.as_deref(), &text)
            .await;

        // Fetch the chat name for the group chat.
        let chat_name = self.fetch_chat_name(&event.event.chat_id).await;

        // Store chat_name and header app_id in last_metadata.
        // header_app_id is used by normalize_inbound_message as the
        // bot_app_id for identity resolution (priority over adapter.app_id).
        {
            let mut meta = self.last_metadata.lock().await;
            meta.clear();
            let name = chat_name.unwrap_or_default();
            if !name.is_empty() {
                meta.insert("chat_name".to_string(), name);
            }
            meta.insert("header_app_id".to_string(), event.header.app_id.clone());
        }

        Ok(Some(NormalizedMessage {
            platform: "feishu".to_string(),
            sender_id: sender_open_id.clone(),
            peer_id: event.event.chat_id,
            content,
            timestamp: chrono::Utc::now().timestamp_millis(),
            message_type: MessageType::from(event.event.message_type.as_str()),
            media_refs,
            thread_id,
            account_id: sender_open_id,
            chat_name: String::new(),
            trace_id: String::new(),
            message_id: String::new(),
            reply_ref: None,
            unavailable_media: Vec::new(),
        }))
    }

    /// Build a `MediaRef` from content JSON using the given key field.
    ///
    /// The `path` field is temporarily set to the platform key (placeholder)
    /// until the media-store implementation provides local persistence.
    /// `media_type` is inferred from the `message_type` parameter.
    /// `size` and `mime` use defaults until download completes.
    fn make_media_ref(
        content: &serde_json::Value,
        key_field: &str,
        message_type: &str,
    ) -> MediaRef {
        let key = content
            .get(key_field)
            .and_then(|k| k.as_str())
            .unwrap_or("")
            .to_string();
        let media_type = MediaType::from(message_type);
        MediaRef {
            key: key.clone(),
            path: key,
            media_type,
            size: 0,
            mime: String::new(),
        }
    }

    /// Extract text and media refs from a message event's content.
    pub(crate) fn extract_message_content(
        message_type: &str,
        content: &serde_json::Value,
    ) -> Result<(String, Vec<MediaRef>), AdapterError> {
        match message_type {
            "text" => Ok((
                content
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string(),
                vec![],
            )),
            "post" => Ok((expand_post_content(content), vec![])),
            "image" => Ok((
                String::new(),
                vec![Self::make_media_ref(content, "image_key", message_type)],
            )),
            "file" | "audio" => Ok((
                String::new(),
                vec![Self::make_media_ref(content, "file_key", message_type)],
            )),
            "sticker" => {
                let emoji_type = content
                    .get("emoji_type")
                    .and_then(|e| e.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("");
                if emoji_type.is_empty() {
                    Ok(("[]".to_string(), vec![]))
                } else {
                    Ok((format!("[{}]", emoji_type), vec![]))
                }
            }
            other => {
                tracing::debug!(message_type = other, "Discarding unsupported message type");
                Err(AdapterError::InvalidPayload(
                    "unsupported message type".to_string(),
                ))
            }
        }
    }
}

#[async_trait]
impl IMAdapter for FeishuAdapter {
    fn name(&self) -> &str {
        "feishu"
    }

    async fn parse_inbound(
        &self,
        payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        let raw: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| AdapterError::InvalidPayload(e.to_string()))?;

        let event_type = Self::extract_event_type(&raw);
        if event_type == "card.action.trigger" {
            return Ok(None);
        }

        if event_type == "reaction.created" {
            return super::events::parse_reaction_event(&raw);
        }

        if event_type == "bot.added" {
            return super::events::parse_bot_added_event(&raw);
        }

        let event: FeishuEvent =
            serde_json::from_value(raw).map_err(|e| AdapterError::InvalidPayload(e.to_string()))?;
        self.parse_message_event(event).await
    }

    async fn parse_card_action(
        &self,
        payload: &[u8],
    ) -> Result<Option<CardActionEvent>, AdapterError> {
        let raw: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| AdapterError::InvalidPayload(e.to_string()))?;

        let event_type = Self::extract_event_type(&raw);
        if event_type != "card.action.trigger" {
            return Ok(None);
        }

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

        let card_event: FeishuCardActionEvent =
            serde_json::from_value(raw).map_err(|e| AdapterError::InvalidPayload(e.to_string()))?;
        self.parse_card_action_event(event_id, app_id, &card_event)
    }

    async fn send_message(
        &self,
        message: &Message,
        root_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        let token = self.get_tenant_token().await.map_err(|e| {
            tracing::warn!(
                receive_id = %message.to,
                error = %e,
                "Feishu token fetch failed"
            );
            e
        })?;
        let content = serde_json::json!({ "text": &message.content }).to_string();
        let resp = self
            .send_msg(&token, &message.to, "text", &content, root_id)
            .await?;
        if resp.code != 0 {
            tracing::warn!(
                receive_id = %message.to,
                code = resp.code,
                msg = %resp.msg,
                "Feishu send error"
            );
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
        let token = self.get_tenant_token().await.map_err(|e| {
            tracing::warn!(
                receive_id = %chat_id,
                error = %e,
                "Feishu card token fetch failed"
            );
            e
        })?;

        let resp = self
            .send_msg(&token, chat_id, "interactive", card_json, root_id)
            .await?;

        if resp.code != 0 {
            tracing::warn!(
                receive_id = %chat_id,
                code = resp.code,
                msg = %resp.msg,
                "Feishu card send error"
            );
            if is_capability_error(resp.code) {
                if let Err(fb_err) = self
                    .try_fallback_to_text(chat_id, card_json, &token, root_id)
                    .await
                {
                    tracing::warn!(
                        receive_id = %chat_id,
                        error = %fb_err,
                        "Text fallback after capability error also failed"
                    );
                    return Err(AdapterError::SendFailed(format!(
                        "Feishu card send error {}: {}",
                        resp.code, resp.msg
                    )));
                }
                // Fallback succeeded — return Ok so mod.rs won't
                // attempt a second fallback (avoids duplicate messages).
                return Ok(());
            }
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
