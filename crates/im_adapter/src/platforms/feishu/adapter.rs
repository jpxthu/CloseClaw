//! Feishu adapter — lark-cli subprocess I/O and event parsing.
use crate::error::AdapterError;
use crate::IMAdapter;
use async_trait::async_trait;
use closeclaw_common::{CardActionEvent, MediaRef, MediaType, MessageType, NormalizedMessage};
use closeclaw_gateway::Message;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use super::post_expand::{expand_post_content, extract_post_media_refs};
use crate::media_store::MediaStore;
use reqwest::Client;
use tokio::sync::Mutex;

// Webhook event types

#[derive(Debug, Deserialize)]
pub(crate) struct FeishuEvent {
    #[allow(dead_code)]
    pub(crate) schema: String,
    pub(crate) header: FeishuHeader,
    pub(crate) event: FeishuMessageEvent,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FeishuHeader {
    #[allow(dead_code)]
    pub(crate) event_id: String,
    #[allow(dead_code)]
    pub(crate) event_type: String,
    #[allow(dead_code)]
    pub(crate) create_time: String,
    #[allow(dead_code)]
    pub(crate) token: String,
    pub(crate) app_id: String,
}

#[derive(Debug, Deserialize)]
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
pub(crate) struct FeishuSender {
    pub(crate) sender_id: FeishuSenderId,
    #[allow(dead_code)]
    pub(crate) sender_type: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FeishuSenderId {
    pub(crate) open_id: String,
}

/// Card action event payload (`card.action.trigger`).
#[derive(Debug, Deserialize)]
pub(crate) struct FeishuCardActionEvent {
    pub(crate) operator: FeishuCardOperator,
    #[allow(dead_code)]
    pub(crate) token: String,
    pub(crate) action: FeishuCardAction,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FeishuCardOperator {
    pub(crate) open_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FeishuCardAction {
    pub(crate) value: Option<serde_json::Value>,
    #[allow(dead_code)]
    pub(crate) tag: Option<String>,
}

/// Default max download size: 50 MB per media resource.
pub(crate) const DEFAULT_MAX_DOWNLOAD_SIZE_BYTES: u64 = 50 * 1024 * 1024;

/// Default lark-cli command name.
const DEFAULT_CLI_COMMAND: &str = "lark-cli";

// Quote helpers

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

// lark-cli response types for message retrieval

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

// lark-cli response type for chat info

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

// lark-cli response type for media download URL

#[derive(Deserialize)]
struct ResourceResp {
    code: i32,
    msg: String,
    data: Option<serde_json::Value>,
}

// FeishuAdapter

/// Feishu adapter implementation.
///
/// All platform communication goes through lark-cli subprocess commands.
/// Credentials are managed by lark-cli via profile — the adapter only
/// stores the profile name and delegates auth to the CLI.
#[derive(Debug, Clone)]
pub struct FeishuAdapter {
    /// lark-cli profile name for credential delegation.
    pub(crate) profile: String,
    /// Metadata produced by the last successful `parse_inbound` call.
    /// Used by `last_parsed_metadata()` to surface platform-specific
    /// fields (e.g. `chat_name`) that were removed from NormalizedMessage.
    pub(crate) last_metadata: Arc<Mutex<HashMap<String, String>>>,
    /// Media storage manager for inbound persistence.
    pub(crate) media_store: Arc<MediaStore>,
    /// Max download size in bytes for a single media file.
    pub(crate) max_download_size_bytes: u64,
    /// Workspace directory for outbound media path resolution.
    pub(crate) workspace_dir: Option<std::path::PathBuf>,
    /// lark-cli command name or path for subprocess execution.
    pub(crate) cli_command: String,
    /// HTTP client for media downloads (media URLs returned by lark-cli).
    http_client: Client,
}

impl FeishuAdapter {
    pub fn new(profile: String, media_store: Arc<MediaStore>) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("FeishuAdapter: failed to build HTTP client");
        Self {
            profile,
            last_metadata: Arc::new(Mutex::new(HashMap::new())),
            media_store,
            max_download_size_bytes: DEFAULT_MAX_DOWNLOAD_SIZE_BYTES,
            workspace_dir: None,
            cli_command: DEFAULT_CLI_COMMAND.to_string(),
            http_client,
        }
    }

    /// Set the workspace directory for outbound media path resolution.
    pub fn with_workspace_dir(mut self, ws: Option<std::path::PathBuf>) -> Self {
        self.workspace_dir = ws;
        self
    }

    /// Fetch the content of a message by its ID via lark-cli.
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

    /// Fetch the raw message item via lark-cli and return (msg_type, content).
    async fn fetch_message_raw(
        &self,
        message_id: &str,
    ) -> Result<Option<(String, String)>, AdapterError> {
        let output = super::send_helpers::run_cli(
            self,
            &["im", "+messages-get", "--message-id", message_id],
        )
        .await?;

        let resp: FeishuGetMessageResponse = serde_json::from_str(&output).map_err(|e| {
            AdapterError::InvalidPayload(format!("lark-cli messages-get invalid JSON: {e}"))
        })?;

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

    /// Fetch the chat (group) name for a given chat_id via lark-cli.
    ///
    /// Returns `Some(name)` on success, or `None` on failure (which logs
    /// a warning and degrades gracefully — chat_name defaults to empty).
    pub async fn fetch_chat_name(&self, chat_id: &str) -> Option<String> {
        let output =
            super::send_helpers::run_cli(self, &["im", "+chats-get", "--chat-id", chat_id])
                .await
                .ok()?;

        let resp: FeishuChatResponse = serde_json::from_str(&output).ok()?;

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

    /// Fetch a temporary download URL for a media resource (image, file, audio)
    /// via lark-cli.
    pub(crate) async fn fetch_media_download_url(
        &self,
        message_id: &str,
        file_key: &str,
        resource_type: &str,
    ) -> Result<String, AdapterError> {
        let output = super::send_helpers::run_cli(
            self,
            &[
                "im",
                "+messages-resources",
                "--message-id",
                message_id,
                "--file-key",
                file_key,
                "--type",
                resource_type,
            ],
        )
        .await?;

        let resp: ResourceResp = serde_json::from_str(&output).map_err(|e| {
            AdapterError::InvalidPayload(format!("lark-cli resources invalid JSON: {e}"))
        })?;

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
            .clone()
            .or(event.event.root_id.clone())
            .or(event.event.parent_id.clone());
        let sender_open_id = event.event.sender.sender_id.open_id.clone();

        let (text, mut media_refs) =
            match Self::extract_message_content(&event.event.message_type, &content) {
                Ok(pair) => pair,
                Err(_) => return Ok(None),
            };

        let unavailable_media = self
            .persist_media_refs(&event, &mut media_refs)
            .await;
        media_refs.retain(|r| !unavailable_media.contains(&r.key));

        if Self::should_discard_message(&event.event.message_type, &text, &media_refs) {
            return Ok(None);
        }

        let content = self
            .prepend_quote_blockquote(original_parent_id.as_deref(), &text)
            .await;
        self.store_event_metadata(&event).await;

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
            unavailable_media,
        }))
    }

    /// Download and persist all media refs, returning unavailable keys.
    async fn persist_media_refs(
        &self,
        event: &FeishuEvent,
        media_refs: &mut [MediaRef],
    ) -> Vec<String> {
        let mut unavailable_media: Vec<String> = Vec::new();
        for r in media_refs.iter_mut() {
            let msg_id = event.event.message_id.as_deref().unwrap_or("");
            match self
                .fetch_media_download_url(msg_id, &r.key, &event.event.message_type)
                .await
            {
                Ok(url) => match self
                    .media_store
                    .download_and_persist(
                        &url,
                        &r.key,
                        &r.media_type,
                        &self.http_client,
                        self.max_download_size_bytes,
                    )
                    .await
                {
                    Ok(persisted) => {
                        r.path = persisted.path;
                        r.size = persisted.size;
                        r.mime = persisted.mime;
                    }
                    Err(e) => {
                        tracing::warn!(key = %r.key, error = %e, "Failed to persist media");
                        unavailable_media.push(r.key.clone());
                    }
                },
                Err(e) => {
                    tracing::warn!(key = %r.key, error = %e, "Failed to fetch media URL");
                    unavailable_media.push(r.key.clone());
                }
            }
        }
        unavailable_media
    }

    /// Determine whether a message should be discarded per design-doc rules.
    fn should_discard_message(
        message_type: &str,
        text: &str,
        media_refs: &[MediaRef],
    ) -> bool {
        match message_type {
            "text" => text.trim().is_empty(),
            "post" => text.trim().is_empty() && media_refs.is_empty(),
            "sticker" => false,
            _ => false,
        }
    }

    /// Store chat_name and header app_id in last_metadata.
    async fn store_event_metadata(&self, event: &FeishuEvent) {
        let chat_name = self.fetch_chat_name(&event.event.chat_id).await;
        let mut meta = self.last_metadata.lock().await;
        meta.clear();
        if let Some(name) = chat_name {
            if !name.is_empty() {
                meta.insert("chat_name".to_string(), name);
            }
        }
        meta.insert("header_app_id".to_string(), event.header.app_id.clone());
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
        let path = key.clone();
        MediaRef {
            key,
            path,
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
            "post" => {
                let media = extract_post_media_refs(content);
                Ok((expand_post_content(content), media))
            }
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

    fn extract_card_ids(raw: &serde_json::Value) -> (String, String) {
        let is_cli = raw.get("type").and_then(|v| v.as_str()).is_some();
        let get = |key: &str| -> String {
            if is_cli {
                raw.get(key)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            } else {
                raw.get("header")
                    .and_then(|h| h.get(key))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            }
        };
        (get("event_id"), get("app_id"))
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

        let event_type = super::process_manager::extract_event_type(&raw);
        if event_type == "card.action.trigger" {
            return Ok(None);
        }

        if event_type == "reaction.created" || event_type == "im.message.reaction.created_v1" {
            return super::events::parse_reaction_event(&raw);
        }

        if event_type == "bot.added" {
            return super::events::parse_bot_added_event(&raw);
        }

        // Determine format: CLI has top-level "type"; webhook has "header.event_type"
        let is_cli = raw.get("type").and_then(|v| v.as_str()).is_some();

        let event = if is_cli {
            super::process_manager::normalize_cli_event(&raw)
                .ok_or_else(|| AdapterError::InvalidPayload("invalid CLI event format".into()))?
        } else {
            serde_json::from_value(raw).map_err(|e| AdapterError::InvalidPayload(e.to_string()))?
        };
        self.parse_message_event(event).await
    }

    async fn parse_card_action(
        &self,
        payload: &[u8],
    ) -> Result<Option<CardActionEvent>, AdapterError> {
        let raw: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| AdapterError::InvalidPayload(e.to_string()))?;
        if super::process_manager::extract_event_type(&raw) != "card.action.trigger" {
            return Ok(None);
        }
        let (event_id, app_id) = Self::extract_card_ids(&raw);
        let card_event: FeishuCardActionEvent =
            serde_json::from_value(raw).map_err(|e| AdapterError::InvalidPayload(e.to_string()))?;
        self.parse_card_action_event(event_id, app_id, &card_event)
    }

    /// Send a text message via lark-cli subprocess.
    async fn send_message(
        &self,
        message: &Message,
        root_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        self.send_msg(&message.to, "text", &message.content, root_id)
            .await
    }

    /// Send an interactive card via lark-cli subprocess.
    async fn send_card_json(
        &self,
        chat_id: &str,
        card_json: &str,
        root_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        match self
            .send_msg(chat_id, "interactive", card_json, root_id)
            .await
        {
            Ok(()) => Ok(()),
            Err(AdapterError::SendFailed(ref msg))
                if super::send_helpers::is_capability_error(msg) =>
            {
                tracing::warn!(
                    receive_id = %chat_id,
                    error = %msg,
                    "Feishu card capability error — falling back to plain text"
                );
                if let Err(fb_err) = self.try_fallback_to_text(chat_id, card_json, root_id).await {
                    tracing::warn!(
                        receive_id = %chat_id,
                        error = %fb_err,
                        "Text fallback after capability error also failed"
                    );
                }
                Ok(())
            }
            Err(e) => {
                tracing::warn!(
                    receive_id = %chat_id,
                    error = %e,
                    "Feishu card send error"
                );
                Err(e)
            }
        }
    }

    async fn validate_signature(&self, _signature: &str, _payload: &[u8]) -> bool {
        // lark-cli event consume handles signature verification.
        // All events received from the subprocess are pre-validated.
        true
    }
}
