//! Feishu (Lark) IM Plugin
//!
//! Unified IM plugin for Feishu messaging platform, wrapping
//! [`FeishuAdapter`] (HTTP I/O) behind a single [`IMPlugin`] implementation.

pub mod adapter;
pub mod cleaner;
#[cfg(test)]
mod cleaner_tests;
pub mod renderer;

use crate::gateway::Message;
use crate::im_adapter::error::AdapterError;
use crate::im_adapter::normalized::NormalizedMessage;
use crate::im_adapter::plugin::{IMPlugin, RenderedOutput};
use crate::im_adapter::IMAdapter;
use crate::llm::types::ContentBlock;
use crate::processor_chain::DslParseResult;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

pub use adapter::CachedToken;
pub use adapter::FeishuAdapter;
use renderer::build_card;
pub use renderer::build_text;
pub use renderer::should_use_card_for_blocks;

/// Unified IM plugin for Feishu.
pub struct FeishuPlugin {
    adapter: Arc<FeishuAdapter>,
}

impl FeishuPlugin {
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
            message_type: "text".to_string(),
            media_refs: vec![],
            quoted_message: None,
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
            return build_text("");
        }

        let has_dsl = dsl_result
            .as_ref()
            .is_some_and(|r| !r.instructions.is_empty());

        if content_blocks.len() == 1 {
            if let ContentBlock::Text(text) = &content_blocks[0] {
                if !has_dsl && !renderer::should_use_card(text, false) {
                    return build_text(text.trim());
                }
            }
        }

        if !should_use_card_for_blocks(content_blocks, has_dsl) {
            return build_text("");
        }

        let (title, elements) = renderer::dispatch_blocks(content_blocks, dsl_result);
        build_card(title, elements)
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

    async fn shutdown(&self) -> Result<(), AdapterError> {
        *self.adapter.cached_token.lock().await = None;
        Ok(())
    }

    fn clean_content(&self, raw: &str) -> String {
        cleaner::clean_feishu_content(raw)
    }
}
