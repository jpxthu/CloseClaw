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
use crate::normalized::{add_code_block_language_hint, normalize_urls};
use crate::IMAdapter;
use async_trait::async_trait;
use closeclaw_common::identity::IdentityResolver;
use closeclaw_common::processor::ContentBlock;
use closeclaw_common::processor::DslParseResult;
use closeclaw_common::{
    AdapterError as CommonAdapterError, CardActionEvent, IMPlugin, NormalizedMessage,
    RenderedOutput,
};
use closeclaw_config::identity::ConfigIdentityResolver;
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
///
/// Identity mapping is loaded from `{config_dir}/config/identity.json`
/// (if the file exists).  A missing or empty file results in no
/// mapping — the fallback uses `sender_id` as `account_id`.
pub async fn register(gateway: &Arc<closeclaw_gateway::Gateway>, config_dir: &str) {
    let app_id = std::env::var("FEISHU_APP_ID").ok();
    let app_secret = std::env::var("FEISHU_APP_SECRET").ok();
    let verification_token = std::env::var("FEISHU_VERIFICATION_TOKEN").ok();
    if let (Some(app_id), Some(app_secret), Some(verification_token)) =
        (app_id, app_secret, verification_token)
    {
        let adapter = Arc::new(FeishuAdapter::new(app_id, app_secret, verification_token));

        // Load identity mapping from config file (best-effort).
        let identity_resolver: Option<Arc<dyn IdentityResolver>> =
            load_identity_resolver(config_dir);

        let plugin: Arc<dyn IMPlugin> = Arc::new(FeishuPlugin::with_identity_resolver(
            adapter,
            identity_resolver,
        ));
        gateway.register_plugin(plugin).await;
        info!("Feishu plugin registered");
    } else {
        info!("Feishu credentials not found in env — Feishu plugin not registered");
    }
}

/// Try to load identity mappings from `{config_dir}/config/identity.json`.
///
/// Returns `Some(Arc<ConfigIdentityResolver>)` when the file exists and
/// contains a valid JSON array, or `None` on any error / missing file.
fn load_identity_resolver(config_dir: &str) -> Option<Arc<dyn IdentityResolver>> {
    let path = std::path::Path::new(config_dir)
        .join("config")
        .join("identity.json");
    match std::fs::read_to_string(&path) {
        Ok(json) => match ConfigIdentityResolver::from_json(&json) {
            Ok(resolver) => {
                if resolver.is_empty() {
                    info!("identity.json loaded but empty — no mappings configured");
                    None
                } else {
                    info!(
                        count = resolver.len(),
                        "identity mapping loaded from {}",
                        path.display()
                    );
                    Some(Arc::new(resolver))
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %path.display(),
                    "failed to parse identity.json — skipping identity mapping"
                );
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            info!("identity.json not found — identity mapping disabled");
            None
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %path.display(),
                "failed to read identity.json — skipping identity mapping"
            );
            None
        }
    }
}

/// Convert im_adapter error to common error.
fn convert_to_common_error(e: AdapterError) -> CommonAdapterError {
    match e {
        AdapterError::InvalidPayload(s) => CommonAdapterError::InvalidPayload(s),
        AdapterError::AuthFailed => CommonAdapterError::AuthFailed,
        AdapterError::SendFailed(s) => CommonAdapterError::SendFailed(s),
        AdapterError::InvalidSignature => CommonAdapterError::InvalidSignature,
        AdapterError::IoError(e) => CommonAdapterError::IoError(e),
        AdapterError::UnsupportedOperation => CommonAdapterError::UnsupportedOperation,
    }
}

/// Unified IM plugin for Feishu.
pub struct FeishuPlugin {
    adapter: Arc<FeishuAdapter>,
    identity_resolver: Option<Arc<dyn IdentityResolver>>,
}

impl FeishuPlugin {
    #[allow(dead_code)]
    pub(crate) fn new(adapter: Arc<FeishuAdapter>) -> Self {
        Self {
            adapter,
            identity_resolver: None,
        }
    }

    /// Create a Feishu plugin with an optional identity resolver.
    #[allow(dead_code)]
    pub(crate) fn with_identity_resolver(
        adapter: Arc<FeishuAdapter>,
        identity_resolver: Option<Arc<dyn IdentityResolver>>,
    ) -> Self {
        Self {
            adapter,
            identity_resolver,
        }
    }

    /// Get the identity resolver for cross-platform account mapping.
    fn identity_resolver(&self) -> Option<&dyn IdentityResolver> {
        self.identity_resolver.as_deref()
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
    ) -> Result<Option<NormalizedMessage>, CommonAdapterError> {
        let mut msg = self
            .adapter
            .parse_inbound(payload)
            .await
            .map_err(convert_to_common_error)?;
        if let Some(ref mut m) = msg {
            m.content = normalize_urls(&m.content);
            m.content = add_code_block_language_hint(&m.content);
            // Apply identity mapping: map (platform, sender_id) → account_id
            if let Some(resolver) = self.identity_resolver() {
                m.account_id = resolver
                    .resolve(&m.platform, &m.sender_id)
                    .unwrap_or(std::mem::take(&mut m.account_id));
            }
        }
        Ok(msg)
    }

    async fn parse_card_action(
        &self,
        payload: &[u8],
    ) -> Result<Option<CardActionEvent>, CommonAdapterError> {
        self.adapter
            .parse_card_action(payload)
            .await
            .map_err(convert_to_common_error)
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
    ) -> Result<(), CommonAdapterError> {
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
                self.adapter
                    .send_message(&message, _thread_id)
                    .await
                    .map_err(convert_to_common_error)
            }
            "interactive" => {
                let card_json = serde_json::to_string(&output.payload)
                    .map_err(|e| CommonAdapterError::SendFailed(e.to_string()))?;
                self.adapter
                    .send_card_json(peer_id, &card_json, _thread_id)
                    .await
                    .map_err(convert_to_common_error)
            }
            _ => Err(CommonAdapterError::UnsupportedOperation),
        }
    }

    async fn shutdown(&self) -> Result<(), CommonAdapterError> {
        *self.adapter.cached_token.lock().await = None;
        Ok(())
    }

    fn clean_content(&self, raw: &str) -> String {
        cleaner::clean_feishu_content(raw)
    }
}
