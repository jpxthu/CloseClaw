//! Feishu (Lark) IM Plugin
//!
//! Unified IM plugin for Feishu messaging platform, wrapping
//! [`FeishuAdapter`] (HTTP I/O) behind a single [`IMPlugin`] implementation.

pub mod adapter;
pub mod cleaner;
#[cfg(test)]
mod cleaner_tests;
pub mod renderer;
pub mod tools;

use crate::error::AdapterError;
use crate::normalized::{add_code_block_language_hint, normalize_urls, NormalizedMessage};
use crate::plugin::{IMPlugin, RenderedOutput};
use crate::streaming::DefaultStreamingRenderer;
use crate::IMAdapter;
use async_trait::async_trait;
use closeclaw_common::processor::ContentBlock;
use closeclaw_common::processor::DslParseResult;
use closeclaw_gateway::Message;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

use super::PlatformEntry;

pub use adapter::CachedToken;
pub use adapter::FeishuAdapter;
use renderer::build_card;
pub use renderer::build_text;
pub use renderer::should_use_card_for_blocks;

inventory::submit!(PlatformEntry {
    name: "feishu",
    register: |gw, cfg| {
        let gw = gw.clone();
        let cfg = cfg.to_string();
        Box::pin(async move { register(&gw, &cfg).await })
    },
});

/// Register the Feishu plugin with the Gateway.
///
/// Reads credentials from environment variables.  If any required
/// variable is missing the plugin is silently not registered.
pub async fn register(gateway: &Arc<closeclaw_gateway::Gateway>, _config_dir: &str) {
    let app_id = std::env::var("FEISHU_APP_ID").ok();
    let app_secret = std::env::var("FEISHU_APP_SECRET").ok();
    let verification_token = std::env::var("FEISHU_VERIFICATION_TOKEN").ok();
    if let (Some(app_id), Some(app_secret), Some(verification_token)) =
        (app_id, app_secret, verification_token)
    {
        let adapter = Arc::new(FeishuAdapter::new(app_id, app_secret, verification_token));
        // Wrap our IMPlugin for the gateway's trait (closeclaw_common::IMPlugin).
        // The bridge module in the main crate provides IMPluginAdapter for this.
        // Here we use a simple wrapper that delegates directly.
        let plugin: Arc<dyn crate::plugin::IMPlugin> = Arc::new(FeishuPlugin::new(adapter));
        let wrapped = wrap_plugin_for_gateway(plugin);
        gateway.register_plugin(wrapped).await;
        info!("Feishu plugin registered");
    } else {
        info!("Feishu credentials not found in env — Feishu plugin not registered");
    }
}

/// Wrap an [`IMPlugin`] (from this crate) into a [`closeclaw_common::IMPlugin`]
/// for registration with the Gateway.
///
/// This is a simple delegation wrapper that converts between the two
/// type systems (they share the same underlying data types from closeclaw-common).
fn wrap_plugin_for_gateway(
    plugin: Arc<dyn crate::plugin::IMPlugin>,
) -> Arc<dyn closeclaw_common::IMPlugin> {
    Arc::new(GatewayPluginWrapper(plugin))
}

/// Wrapper that adapts our [`IMPlugin`] to the gateway's [`closeclaw_common::IMPlugin`].
struct GatewayPluginWrapper(Arc<dyn crate::plugin::IMPlugin>);

#[async_trait]
impl closeclaw_common::IMPlugin for GatewayPluginWrapper {
    fn platform(&self) -> &str {
        self.0.platform()
    }

    async fn parse_inbound(
        &self,
        payload: &[u8],
    ) -> Result<
        Option<closeclaw_common::im_plugin::NormalizedMessage>,
        closeclaw_common::im_plugin::AdapterError,
    > {
        self.0
            .parse_inbound(payload)
            .await
            .map(|opt| opt.map(convert_to_common_normalized))
            .map_err(convert_to_common_error)
    }

    async fn validate_signature(&self, signature: &str, payload: &[u8]) -> bool {
        self.0.validate_signature(signature, payload).await
    }

    async fn send(
        &self,
        output: &closeclaw_common::im_plugin::RenderedOutput,
        peer_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), closeclaw_common::im_plugin::AdapterError> {
        let main_output = RenderedOutput {
            msg_type: output.msg_type.clone(),
            payload: output.payload.clone(),
        };
        self.0
            .send(&main_output, peer_id, thread_id)
            .await
            .map_err(convert_to_common_error)
    }

    fn clean_content(&self, raw: &str) -> String {
        self.0.clean_content(raw)
    }

    async fn init(&self) -> Result<(), closeclaw_common::im_plugin::AdapterError> {
        self.0.init().await.map_err(convert_to_common_error)
    }

    async fn shutdown(&self) -> Result<(), closeclaw_common::im_plugin::AdapterError> {
        self.0.shutdown().await.map_err(convert_to_common_error)
    }

    fn render(
        &self,
        content_blocks: &[closeclaw_common::processor::ContentBlock],
        dsl_result: Option<&closeclaw_common::processor::DslParseResult>,
    ) -> closeclaw_common::im_plugin::RenderedOutput {
        let result = self.0.render(content_blocks, dsl_result);
        closeclaw_common::im_plugin::RenderedOutput {
            msg_type: result.msg_type,
            payload: result.payload,
        }
    }

    fn handle_stream_event(
        &self,
        event: closeclaw_common::processor::StreamEvent,
    ) -> closeclaw_common::im_plugin::StreamingOutput {
        let result = self.0.handle_stream_event(event);
        closeclaw_common::im_plugin::StreamingOutput {
            text_messages: result.text_messages,
            render_blocks: result.render_blocks,
        }
    }

    fn flush_stream(&self) -> closeclaw_common::im_plugin::StreamingOutput {
        let result = self.0.flush_stream();
        closeclaw_common::im_plugin::StreamingOutput {
            text_messages: result.text_messages,
            render_blocks: result.render_blocks,
        }
    }
}

fn convert_to_common_error(e: AdapterError) -> closeclaw_common::im_plugin::AdapterError {
    match e {
        AdapterError::InvalidPayload(s) => {
            closeclaw_common::im_plugin::AdapterError::InvalidPayload(s)
        }
        AdapterError::AuthFailed => closeclaw_common::im_plugin::AdapterError::AuthFailed,
        AdapterError::SendFailed(s) => closeclaw_common::im_plugin::AdapterError::SendFailed(s),
        AdapterError::InvalidSignature => {
            closeclaw_common::im_plugin::AdapterError::InvalidSignature
        }
        AdapterError::IoError(e) => closeclaw_common::im_plugin::AdapterError::IoError(e),
        AdapterError::UnsupportedOperation => {
            closeclaw_common::im_plugin::AdapterError::UnsupportedOperation
        }
        AdapterError::MediaDownloadFailed(s) => {
            closeclaw_common::im_plugin::AdapterError::SendFailed(s)
        }
    }
}

fn convert_to_common_normalized(
    m: NormalizedMessage,
) -> closeclaw_common::im_plugin::NormalizedMessage {
    closeclaw_common::im_plugin::NormalizedMessage {
        platform: m.platform,
        sender_id: m.sender_id,
        peer_id: m.peer_id,
        content: m.content,
        timestamp: m.timestamp,
        message_type: m.message_type,
        media_refs: m
            .media_refs
            .into_iter()
            .map(|r| closeclaw_common::im_plugin::MediaRef {
                key: r.key,
                url: r.url,
            })
            .collect(),
        quoted_message: m
            .quoted_message
            .map(|q| closeclaw_common::im_plugin::QuotedMessage {
                content: q.content,
                sender_id: q.sender_id,
            }),
        thread_id: m.thread_id,
        account_id: m.account_id,
        card_action: m.card_action,
    }
}

/// Unified IM plugin for Feishu.
pub struct FeishuPlugin {
    adapter: Arc<FeishuAdapter>,
    renderer: std::sync::Mutex<DefaultStreamingRenderer>,
}

impl FeishuPlugin {
    pub(crate) fn new(adapter: Arc<FeishuAdapter>) -> Self {
        Self {
            adapter,
            renderer: std::sync::Mutex::new(DefaultStreamingRenderer::new()),
        }
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
        let mut msg = self.adapter.handle_webhook(payload).await?;
        if let Some(ref mut m) = msg {
            m.content = normalize_urls(&m.content);
            m.content = add_code_block_language_hint(&m.content);
        }
        Ok(msg)
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

    fn streaming_renderer(&self) -> &std::sync::Mutex<DefaultStreamingRenderer> {
        &self.renderer
    }
}
