//! Feishu (Lark) IM Adapter
//!
//! Implements IMAdapter for Feishu messaging platform.

use super::{AdapterError, IMAdapter, IMPlugin, NormalizedMessage};
use crate::gateway::Message;
use crate::im_adapter::code_block::{parse_content_segments, ContentSegment};
use crate::llm::types::ContentBlock;
use crate::processor_chain::dsl_parser::{DslInstruction, DslParseResult};
use crate::renderer::RenderedOutput;
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
    /// Thread ID — direct thread identifier from Feishu.
    #[serde(default)]
    thread_id: Option<String>,
    /// Root message ID of the thread — used as fallback for thread_id.
    #[serde(default)]
    root_id: Option<String>,
    /// Parent message ID — second fallback for thread_id.
    #[serde(default)]
    parent_id: Option<String>,
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

/// Feishu card action event payload (card.action.trigger)
///
/// Fields are populated by serde during deserialization and consumed
/// indirectly via [`FeishuAdapter::handle_card_action`]; not all fields
/// are read directly in Rust code.
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // fields used by serde deserialization, consumed indirectly
struct FeishuCardActionEvent {
    operator: FeishuCardOperator,
    token: String,
    action: FeishuCardAction,
}

/// Operator who triggered the card action.
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // fields used by serde deserialization, consumed indirectly
struct FeishuCardOperator {
    open_id: String,
}

/// Action payload from a Feishu card button click.
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // fields used by serde deserialization, consumed indirectly
struct FeishuCardAction {
    value: Option<serde_json::Value>,
    tag: Option<String>,
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

    /// Handle a card.action.trigger event.
    ///
    /// Returns `Some(Message)` for recognized actions (e.g. `forceful_shutdown`),
    /// or `None` for unknown/unsupported actions.
    fn handle_card_action(
        &self,
        event_id: String,
        app_id: String,
        card_event: &FeishuCardActionEvent,
    ) -> Result<Option<Message>, AdapterError> {
        let action_value = card_event
            .action
            .value
            .as_ref()
            .and_then(|v| v.get("action"))
            .and_then(|a| a.as_str());

        match action_value {
            Some("forceful_shutdown") => {
                let mut metadata = HashMap::from([
                    ("account_id".to_string(), app_id),
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
                Ok(Some(Message {
                    id: event_id,
                    from: card_event.operator.open_id.clone(),
                    to: String::new(),
                    content: "/__card_action:forceful_shutdown".to_string(),
                    channel: "feishu".to_string(),
                    timestamp: chrono::Utc::now().timestamp(),
                    metadata,
                    thread_id: None,
                }))
            }
            _ => Ok(None),
        }
    }

    /// Parse a regular message event (im.message.receive_v1) into a Message.
    fn parse_message_event(&self, event: FeishuEvent) -> Result<Message, AdapterError> {
        let content: serde_json::Value = serde_json::from_str(&event.event.content)
            .map_err(|e| AdapterError::InvalidPayload(e.to_string()))?;

        let text = content
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();

        // Extract thread_id with priority: thread_id > root_id > parent_id
        let thread_id = event
            .event
            .thread_id
            .or(event.event.root_id)
            .or(event.event.parent_id);

        let mut metadata = HashMap::from([("account_id".to_string(), event.header.app_id.clone())]);
        if let Some(tid) = thread_id {
            metadata.insert("thread_id".to_string(), tid);
        }

        Ok(Message {
            id: event.header.event_id,
            from: event.event.sender.sender_id.open_id,
            to: String::new(), // Will be filled by gateway
            content: text,
            channel: "feishu".to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            metadata,
            thread_id: None,
        })
    }
}

#[async_trait]
impl IMAdapter for FeishuAdapter {
    fn name(&self) -> &str {
        "feishu"
    }

    async fn handle_webhook(&self, payload: &[u8]) -> Result<Option<Message>, AdapterError> {
        // First, peek at the event_type to decide how to parse.
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
                // Regular message event — parse as before.
                let event: FeishuEvent = serde_json::from_value(raw)
                    .map_err(|e| AdapterError::InvalidPayload(e.to_string()))?;
                Ok(Some(self.parse_message_event(event)?))
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

        let mut url = format!("{}/im/v1/messages?receive_id_type=open_id", FEISHU_API_BASE);
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

        let mut url = format!("{}/im/v1/messages?receive_id_type=chat_id", FEISHU_API_BASE);
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

// ---------------------------------------------------------------------------
// Card types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
struct CardPayload {
    msg_type: String,
    card: Card,
}

#[derive(Debug, Clone, Serialize)]
struct Card {
    header: Option<CardHeader>,
    elements: Vec<CardElement>,
}

#[derive(Debug, Clone, Serialize)]
struct CardHeader {
    title: String,
    template: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "tag")]
enum CardElement {
    #[serde(rename = "markdown")]
    Markdown { content: String },
    #[serde(rename = "hr")]
    Hr,
    #[serde(rename = "action")]
    Action { actions: Vec<CardAction> },
    #[serde(rename = "note")]
    Note { elements: Vec<CardNoteElement> },
}

#[derive(Debug, Clone, Serialize)]
struct CardNoteElement {
    tag: String,
    content: String,
}

impl CardNoteElement {
    fn plain_text(content: impl Into<String>) -> Self {
        Self {
            tag: "plain_text".into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct CardAction {
    tag: String,
    text: CardText,
    #[serde(rename = "type")]
    action_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct CardText {
    tag: String,
    content: String,
}

// ---------------------------------------------------------------------------
// FeishuPlugin
// ---------------------------------------------------------------------------

/// Unified IM plugin for Feishu, wrapping a [`FeishuAdapter`] (HTTP I/O)
/// behind a single [`IMPlugin`] implementation. The gateway registers one
/// instance per platform via `IMPlugin::platform()` and routes all inbound /
/// outbound calls through it.
pub struct FeishuPlugin {
    adapter: Arc<FeishuAdapter>,
}

impl FeishuPlugin {
    /// Build a new [`FeishuPlugin`] from a Feishu adapter.
    pub(crate) fn new(adapter: Arc<FeishuAdapter>) -> Self {
        Self { adapter }
    }
}

#[async_trait]
impl IMPlugin for FeishuPlugin {
    fn platform(&self) -> &str {
        "feishu"
    }

    async fn parse_inbound(
        &self,
        payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        let message = match self.adapter.handle_webhook(payload).await? {
            Some(m) => m,
            None => return Ok(None),
        };
        Ok(Some(NormalizedMessage {
            platform: message.channel,
            sender_id: message.from,
            peer_id: message.to,
            content: message.content,
            timestamp: message.timestamp,
            thread_id: message.metadata.get("thread_id").cloned(),
            account_id: message.metadata.get("account_id").cloned(),
            card_action: message.metadata.get("card_action").map(|v| v == "true"),
        }))
    }

    async fn validate_signature(&self, signature: &str, payload: &[u8]) -> bool {
        self.adapter.validate_signature(signature, payload).await
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        if content_blocks.is_empty() {
            return Self::build_text("");
        }

        let has_dsl = dsl_result
            .as_ref()
            .is_some_and(|r| !r.instructions.is_empty());

        if content_blocks.len() == 1 {
            if let ContentBlock::Text(text) = &content_blocks[0] {
                if !has_dsl && !Self::should_use_card(text, false) {
                    return Self::build_text(text.trim());
                }
            }
        }

        if !Self::should_use_card_for_blocks(content_blocks, has_dsl) {
            return Self::build_text("");
        }

        let (title, elements) = Self::dispatch_blocks(content_blocks, dsl_result);
        Self::build_card(title, elements)
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        match output.msg_type.as_str() {
            "text" => {
                let text = output
                    .payload
                    .get("content")
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let message = Message {
                    id: String::new(),
                    from: String::new(),
                    to: peer_id.to_string(),
                    content: text.to_string(),
                    channel: "feishu".to_string(),
                    timestamp: chrono::Utc::now().timestamp(),
                    metadata: HashMap::new(),
                    thread_id: None,
                };
                self.adapter.send_message(&message, _thread_id).await
            }
            "interactive" => {
                let card_json = serde_json::to_string(&output.payload)
                    .map_err(|e| AdapterError::SendFailed(e.to_string()))?;
                self.adapter
                    .send_card_json(peer_id, &card_json, _thread_id)
                    .await
            }
            _ => Err(AdapterError::UnsupportedOperation),
        }
    }

    async fn close_inbound(&self) -> Result<(), AdapterError> {
        // Feishu uses stateless HTTP webhooks — nothing to disconnect.
        // Clear the cached tenant token to release resources.
        *self.adapter.cached_token.lock().await = None;
        Ok(())
    }

    async fn close_outbound(&self) -> Result<(), AdapterError> {
        // Feishu sends via stateless HTTP — no queue to drain.
        // Clear the cached tenant token to release resources.
        *self.adapter.cached_token.lock().await = None;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FeishuPlugin — card-building helpers
// ---------------------------------------------------------------------------

impl FeishuPlugin {
    /// Returns true when content needs a card (has DSL, header, newlines, or
    /// inline formatting).
    fn should_use_card(content: &str, has_dsl: bool) -> bool {
        let md = content.trim();
        if md.is_empty() {
            return false;
        }
        if has_dsl || md.starts_with('#') || md.contains('\n') {
            return true;
        }
        contains_inline(md)
    }

    /// Returns true when the structured content blocks warrant an interactive
    /// card.
    fn should_use_card_for_blocks(content_blocks: &[ContentBlock], has_dsl: bool) -> bool {
        if content_blocks.is_empty() {
            return false;
        }
        if has_dsl {
            return true;
        }
        let has_non_text = content_blocks
            .iter()
            .any(|b| !matches!(b, ContentBlock::Text(_)));
        if content_blocks.len() > 1 || has_non_text {
            return true;
        }
        if let ContentBlock::Text(text) = &content_blocks[0] {
            return Self::should_use_card(text, false);
        }
        true
    }

    /// Extracts `# Title` from first line.
    fn extract_header(content: &str) -> (Option<String>, String) {
        let trimmed = content.trim_start();
        if !trimmed.starts_with("# ") {
            return (None, content.to_string());
        }
        let end = trimmed.find('\n').unwrap_or(trimmed.len());
        let title = trimmed[2..end].trim().to_string();
        let rest = if end < trimmed.len() {
            trimmed[end + 1..].trim_end().to_string()
        } else {
            String::new()
        };
        (Some(title), rest)
    }

    /// Converts markdown to card elements.
    fn to_elements(content: &str) -> Vec<CardElement> {
        parse_content_segments(content)
            .into_iter()
            .map(|seg| match seg {
                ContentSegment::Markdown(text) => CardElement::Markdown { content: text },
                ContentSegment::Hr => CardElement::Hr,
                ContentSegment::CodeBlock { language, code } => CardElement::Markdown {
                    content: if language.is_empty() {
                        format!("```\n{code}\n```")
                    } else {
                        format!("```{language}\n{code}\n```")
                    },
                },
            })
            .collect()
    }

    /// Render a Thinking block as a Feishu markdown quote block.
    fn render_thinking_block(content: &str) -> CardElement {
        let quoted = content
            .lines()
            .map(|line| {
                if line.is_empty() {
                    ">".to_string()
                } else {
                    format!("> {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let body = if quoted.is_empty() {
            "> 💭 Thinking".to_string()
        } else {
            format!("> 💭 Thinking\n{quoted}")
        };
        CardElement::Markdown { content: body }
    }

    /// Render a ToolUse block as a Feishu `note` element.
    fn render_tool_use_block(name: &str, input: &str) -> CardElement {
        const INPUT_PREVIEW_LIMIT: usize = 200;
        let preview: String = input.chars().take(INPUT_PREVIEW_LIMIT).collect();
        let truncated = input.chars().count() > INPUT_PREVIEW_LIMIT;
        let summary = if truncated {
            format!("{preview}…")
        } else {
            preview
        };
        let line = if summary.is_empty() {
            format!("🔧 {name}")
        } else {
            format!("🔧 {name}: {summary}")
        };
        CardElement::Note {
            elements: vec![CardNoteElement::plain_text(line)],
        }
    }

    /// Render a ToolResult block as a markdown element.
    fn render_tool_result_block(content: &str) -> CardElement {
        const RESULT_LIMIT: usize = 2000;
        let char_count = content.chars().count();
        if char_count <= RESULT_LIMIT {
            return CardElement::Markdown {
                content: format!("**Result**\n```\n{content}\n```"),
            };
        }
        let preview: String = content.chars().take(RESULT_LIMIT).collect();
        CardElement::Markdown {
            content: format!(
                "**Result**\n```\n{preview}\n```\n\n\
                 _结果过长，已截断（{char_count} 字符，显示前 {RESULT_LIMIT}）_"
            ),
        }
    }

    /// Dispatch content blocks by type, producing a title and card elements.
    fn dispatch_blocks(
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> (Option<String>, Vec<CardElement>) {
        let mut title: Option<String> = None;
        let mut elements: Vec<CardElement> = Vec::new();

        for block in content_blocks {
            match block {
                ContentBlock::Text(text) => {
                    if title.is_none() {
                        let (t, body) = Self::extract_header(text.trim());
                        title = t;
                        elements.extend(Self::to_elements(&body));
                    } else {
                        elements.extend(Self::to_elements(text.trim()));
                    }
                }
                ContentBlock::Thinking(content) => {
                    elements.push(Self::render_thinking_block(content));
                }
                ContentBlock::ToolUse { name, input, .. } => {
                    elements.push(Self::render_tool_use_block(name, input));
                }
                ContentBlock::ToolResult { content, .. } => {
                    elements.push(Self::render_tool_result_block(content));
                }
                ContentBlock::Image(name) => {
                    elements.extend(Self::to_elements(&format!("[image: {name}]")));
                }
                ContentBlock::Audio(name) => {
                    elements.extend(Self::to_elements(&format!("[audio: {name}]")));
                }
                ContentBlock::File(name) => {
                    elements.extend(Self::to_elements(&format!("[file: {name}]")));
                }
            }
        }

        if let Some(r) = dsl_result {
            elements.extend(Self::render_buttons(&r.instructions));
        }

        (title, elements)
    }

    /// Renders DSL instructions as buttons.
    fn render_buttons(instructions: &[DslInstruction]) -> Vec<CardElement> {
        if instructions.is_empty() {
            return Vec::new();
        }
        let has_primary = instructions
            .iter()
            .any(|i| matches!(i, DslInstruction::Button { .. }));
        let mut actions = Vec::new();
        let mut seen = false;

        for inst in instructions {
            let DslInstruction::Button { label, .. } = inst else {
                continue;
            };
            let bt = if has_primary && !seen {
                seen = true;
                "primary"
            } else {
                "default"
            };
            actions.push(CardAction {
                tag: "button".into(),
                text: CardText {
                    tag: "plain_text".into(),
                    content: label.clone(),
                },
                action_type: bt.into(),
                url: None,
            });
        }
        vec![CardElement::Action { actions }]
    }

    /// Builds an interactive card [`RenderedOutput`].
    fn build_card(title: Option<String>, elements: Vec<CardElement>) -> RenderedOutput {
        let header = title.map(|t| CardHeader {
            title: t,
            template: "blue".into(),
        });
        let card = Card { header, elements };
        let payload = CardPayload {
            msg_type: "interactive".into(),
            card,
        };
        RenderedOutput {
            msg_type: "interactive".into(),
            payload: serde_json::to_value(&payload).unwrap_or(serde_json::Value::Null),
        }
    }

    /// Returns a plain text [`RenderedOutput`].
    fn build_text(content: &str) -> RenderedOutput {
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({
                "msg_type": "text",
                "content": { "text": content }
            }),
        }
    }
}

fn contains_inline(s: &str) -> bool {
    s.contains("**")
        || s.contains("__")
        || s.contains('*')
        || s.contains('_')
        || s.contains('`')
        || (s.contains('[') && s.contains("]("))
}
