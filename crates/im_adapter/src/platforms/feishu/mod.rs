//! Feishu (Lark) IM Plugin
//!
//! Unified IM plugin for Feishu messaging platform, wrapping
//! [`FeishuAdapter`] (HTTP I/O) behind a single [`IMPlugin`] implementation.

mod adapter;
#[cfg(test)]
mod adapter_sticker_tests;
#[cfg(test)]
mod adapter_tests;
pub mod cleaner;
#[cfg(test)]
mod cleaner_tests;
mod events;
#[cfg(test)]
mod events_tests;
#[cfg(test)]
mod feishu_adapter_tests;
#[cfg(test)]
mod feishu_tests;
mod post_expand;
pub mod renderer;
#[cfg(test)]
mod send_fallback_tests;
mod send_helpers;
#[cfg(test)]
mod send_warn_tests;
#[cfg(test)]
mod style_tests;
mod text_style;
pub mod tools;
#[cfg(test)]
mod trace_id_tests;

use crate::error::AdapterError;
use crate::normalized::{add_code_block_language_hint, normalize_urls};
use crate::IMAdapter;
use async_trait::async_trait;
use closeclaw_common::identity::IdentityResolver;
use closeclaw_common::processor::{ContentBlock, DslParseResult};
use closeclaw_common::streaming::{CodeBlockMode, DefaultStreamingRenderer};
use closeclaw_common::{
    AdapterError as CommonAdapterError, CardActionEvent, IMPlugin, NormalizedMessage,
    RenderedOutput,
};
use closeclaw_config::identity::ConfigIdentityResolver;
use closeclaw_debug_log::DebugLog;
use closeclaw_gateway::Message;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

use super::PlatformEntry;

pub use adapter::CachedToken;
pub use adapter::FeishuAdapter;
use renderer::build_card;
pub use renderer::build_text;
use renderer::extract_card_plain_text;
pub use renderer::should_use_card_for_blocks;

// Re-export adapter internals for test modules.
#[cfg(test)]
pub(crate) use adapter::{
    is_capability_error, truncate_to_500, FeishuEvent, FeishuHeader, FeishuMessageEvent,
    FeishuSender, FeishuSenderId, FEISHU_API_BASE,
};
#[cfg(test)]
pub(crate) use post_expand::expand_post_content;

inventory::submit!(PlatformEntry {
    name: "feishu",
    register: |gw, cfg| {
        let gw = gw.clone();
        let cfg = cfg.to_string();
        Box::pin(async move { register(&gw, &cfg).await })
    },
});

/// Root platforms configuration loaded from `platforms.json`.
///
/// Each key is a platform name and `enabled` controls whether
/// the platform plugin is registered at startup.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub(crate) struct PlatformsConfig {
    platforms: HashMap<String, PlatformEnabledEntry>,
}

/// A single platform entry in `platforms.json`.
#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct PlatformEnabledEntry {
    #[serde(default)]
    enabled: bool,
}

impl PlatformsConfig {
    /// Check whether a platform is explicitly enabled.
    fn is_enabled(&self, platform: &str) -> bool {
        self.platforms.get(platform).is_some_and(|e| e.enabled)
    }
}

/// Load `{config_dir}/config/platforms.json`.
///
/// Returns an empty config when the file is missing or unparseable.
pub(crate) fn load_platforms_config(config_dir: &str) -> PlatformsConfig {
    let path = std::path::Path::new(config_dir)
        .join("config")
        .join("platforms.json");
    match std::fs::read_to_string(&path) {
        Ok(json) => match serde_json::from_str::<PlatformsConfig>(&json) {
            Ok(cfg) => cfg,
            Err(e) => {
                warn!(
                    error = %e,
                    path = %path.display(),
                    "failed to parse platforms.json — all platforms disabled"
                );
                PlatformsConfig::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            info!("platforms.json not found — all platforms disabled");
            PlatformsConfig::default()
        }
        Err(e) => {
            warn!(
                error = %e,
                path = %path.display(),
                "failed to read platforms.json — all platforms disabled"
            );
            PlatformsConfig::default()
        }
    }
}

/// Register the Feishu plugin with the Gateway.
///
/// First checks `{config_dir}/config/platforms.json` for an explicit
/// enable flag.  If the platform is not listed or disabled the plugin
/// is silently not registered.  When enabled, credentials are read
/// from environment variables; missing env vars emit a warning.
///
/// Identity mapping is loaded from `{config_dir}/config/accounts.json`
/// (if the file exists).  A missing or empty file results in no
/// mapping — the fallback uses `sender_id` as `account_id`.
pub async fn register(gateway: &Arc<closeclaw_gateway::Gateway>, config_dir: &str) {
    let platforms = load_platforms_config(config_dir);
    if !platforms.is_enabled("feishu") {
        info!("feishu not enabled in platforms.json — skipping");
        return;
    }

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

        let mut plugin = FeishuPlugin::with_identity_resolver(adapter, identity_resolver);

        // Inject DebugLog from Gateway (if configured).
        if let Some(debug_log) = gateway.get_debug_log() {
            plugin.set_debug_log(Arc::new(debug_log));
        }

        let plugin: Arc<dyn IMPlugin> = Arc::new(plugin);
        gateway.register_plugin(plugin).await;
        info!("Feishu plugin registered");
    } else {
        warn!("feishu enabled in platforms.json but credentials missing in env — skipping");
    }
}

/// Try to load identity mappings from `{config_dir}/config/accounts.json`.
///
/// Returns `Some(Arc<ConfigIdentityResolver>)` when the file exists and
/// contains a valid JSON object with an `accounts` array, or `None` on
/// any error / missing file.
pub(crate) fn load_identity_resolver(config_dir: &str) -> Option<Arc<dyn IdentityResolver>> {
    use closeclaw_config::AccountsConfigData;

    let path = std::path::Path::new(config_dir)
        .join("config")
        .join("accounts.json");
    match std::fs::read_to_string(&path) {
        Ok(json) => match AccountsConfigData::from_json_str(&json) {
            Ok(accounts_data) => {
                let resolver = ConfigIdentityResolver::new(accounts_data.accounts);
                if resolver.is_empty() {
                    info!("accounts.json loaded but empty — no mappings configured");
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
                    "failed to parse accounts.json — skipping identity mapping"
                );
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            info!("accounts.json not found — identity mapping disabled");
            None
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %path.display(),
                "failed to read accounts.json — skipping identity mapping"
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
    streaming_renderer: std::sync::Mutex<DefaultStreamingRenderer>,
    /// Debug log framework instance for structured event logging.
    debug_log: Option<Arc<DebugLog>>,
}

impl FeishuPlugin {
    #[allow(dead_code)]
    pub(crate) fn new(adapter: Arc<FeishuAdapter>) -> Self {
        Self {
            adapter,
            identity_resolver: None,
            streaming_renderer: std::sync::Mutex::new(
                DefaultStreamingRenderer::new().with_code_block_mode(CodeBlockMode::WholeBlock),
            ),
            debug_log: None,
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
            streaming_renderer: std::sync::Mutex::new(
                DefaultStreamingRenderer::new().with_code_block_mode(CodeBlockMode::WholeBlock),
            ),
            debug_log: None,
        }
    }

    /// Inject a [`DebugLog`] instance for structured event logging.
    pub fn set_debug_log(&mut self, debug_log: Arc<DebugLog>) {
        self.debug_log = Some(debug_log);
    }

    /// Get the identity resolver for cross-platform account mapping.
    fn identity_resolver(&self) -> Option<&dyn IdentityResolver> {
        self.identity_resolver.as_deref()
    }

    /// Fallback: extract plain text from an interactive card and send
    /// via text message API.  Logs warnings on failure and always
    /// returns `Ok(())` so the Agent keeps running.
    async fn send_interactive_fallback(
        &self,
        peer_id: &str,
        output: &RenderedOutput,
        thread_id: Option<&str>,
    ) {
        let plain_text = extract_card_plain_text(&output.payload);
        if plain_text.is_empty() {
            warn!(
                peer_id = %peer_id,
                "No extractable text in card payload — returning Ok(())"
            );
            return;
        }
        let fallback = Self::make_text_message(peer_id, &plain_text);
        if let Err(e2) = self.adapter.send_message(&fallback, thread_id).await {
            warn!(
                peer_id = %peer_id,
                error = %e2,
                "Feishu text fallback also failed — returning Ok(()) per design doc"
            );
        }
    }

    /// Build a text-mode [`Message`] targeting `peer_id`.
    fn make_text_message(peer_id: &str, text: &str) -> Message {
        Message {
            id: String::new(),
            from: String::new(),
            to: peer_id.to_string(),
            content: text.to_string(),
            channel: "feishu".to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            metadata: HashMap::new(),
            thread_id: None,
            platform: None,
            dsl_result: None,
            content_blocks: None,
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

    fn last_parsed_metadata(&self) -> HashMap<String, String> {
        // Delegate to the inner adapter's last_metadata (blocking lock).
        // This is safe because last_parsed_metadata is called synchronously
        // after parse_inbound in the gateway's inbound queue consumer.
        match self.adapter.last_metadata.try_lock() {
            Ok(guard) => guard.clone(),
            Err(_) => {
                warn!("last_parsed_metadata: try_lock failed; returning empty map");
                HashMap::new()
            }
        }
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

        let (title, elements) = renderer::dispatch_blocks(content_blocks, dsl_result, true);
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
                let message = Self::make_text_message(peer_id, text);
                if let Err(e) = self.adapter.send_message(&message, _thread_id).await {
                    warn!(
                        peer_id = %peer_id,
                        error = %e,
                        "Feishu text send failed — returning Ok(()) per design doc"
                    );
                    return Ok(());
                }
                Ok(())
            }
            "interactive" => {
                let card_json = serde_json::to_string(&output.payload)
                    .map_err(|e| CommonAdapterError::SendFailed(e.to_string()))?;
                match self
                    .adapter
                    .send_card_json(peer_id, &card_json, _thread_id)
                    .await
                {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        warn!(
                            peer_id = %peer_id,
                            error = %e,
                            "Feishu interactive card send failed — falling back to plain text"
                        );
                        self.send_interactive_fallback(peer_id, output, _thread_id)
                            .await;
                        Ok(())
                    }
                }
            }
            _ => Err(CommonAdapterError::UnsupportedOperation),
        }
    }

    async fn shutdown(&self) -> Result<(), CommonAdapterError> {
        *self.adapter.cached_token.lock().await = None;
        Ok(())
    }

    fn streaming_renderer(&self) -> Option<&std::sync::Mutex<DefaultStreamingRenderer>> {
        Some(&self.streaming_renderer)
    }

    fn clean_content(&self, raw: &str) -> String {
        cleaner::clean_feishu_content(raw)
    }
}
