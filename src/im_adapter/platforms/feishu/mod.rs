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

use crate::gateway::Message;
use crate::im_adapter::error::AdapterError;
use crate::im_adapter::normalized::{
    add_code_block_language_hint, normalize_urls, NormalizedMessage,
};
use crate::im_adapter::plugin::{IMPlugin, RenderedOutput};
use crate::im_adapter::streaming::DefaultStreamingRenderer;
use crate::im_adapter::IMAdapter;
use crate::llm::types::ContentBlock;
use crate::processor_chain::DslParseResult;
use async_trait::async_trait;
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
pub async fn register(gateway: &Arc<crate::gateway::Gateway>, _config_dir: &str) {
    let app_id = std::env::var("FEISHU_APP_ID").ok();
    let app_secret = std::env::var("FEISHU_APP_SECRET").ok();
    let verification_token = std::env::var("FEISHU_VERIFICATION_TOKEN").ok();
    if let (Some(app_id), Some(app_secret), Some(verification_token)) =
        (app_id, app_secret, verification_token)
    {
        let adapter = Arc::new(FeishuAdapter::new(app_id, app_secret, verification_token));
        let plugin: Arc<dyn crate::im::IMPlugin> = Arc::new(FeishuPlugin::new(adapter));
        gateway
            .register_plugin(crate::bridge::IMPluginAdapter::wrap(plugin))
            .await;
        info!("Feishu plugin registered");
    } else {
        info!("Feishu credentials not found in env — Feishu plugin not registered");
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
