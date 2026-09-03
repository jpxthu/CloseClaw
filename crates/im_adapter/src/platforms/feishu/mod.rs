//! Feishu (Lark) IM Plugin
//!
//! Unified IM plugin for Feishu messaging platform, wrapping
//! [`FeishuAdapter`] (HTTP I/O) behind a single [`IMPlugin`] implementation.

mod adapter;
#[cfg(test)]
mod adapter_emoji_tests;
#[cfg(test)]
mod adapter_sticker_tests;
#[cfg(test)]
mod adapter_tests;
pub(crate) mod card_media_fallback;
#[cfg(test)]
mod card_media_tests;
pub(crate) mod cardkit_streaming;
pub mod cleaner;
#[cfg(test)]
mod cleaner_tests;
#[cfg(test)]
mod credential_isolation_tests;
#[cfg(test)]
mod debug_log_tests;
mod events;
#[cfg(test)]
mod events_tests;
#[cfg(test)]
mod feishu_adapter_tests;
#[cfg(test)]
mod feishu_tests;
#[cfg(test)]
mod identity_isolation_tests;
#[cfg(test)]
mod media_filter_tests;
#[cfg(test)]
mod normalize_cli_event_tests;
mod outbound_media;
#[cfg(test)]
mod outbound_media_tests;
mod post_expand;
pub(crate) mod process_manager;
#[cfg(test)]
mod process_manager_tests;
pub mod renderer;
#[cfg(test)]
mod send_fallback_tests;
mod send_helpers;
#[cfg(test)]
mod send_warn_tests;
#[cfg(test)]
mod step1_8_gap_tests;
pub(crate) mod streaming_send;
#[cfg(test)]
mod style_tests;
mod text_style;
pub mod tools;
#[cfg(test)]
mod trace_id_tests;
#[cfg(test)]
mod try_resolve_media_path_tests;

use self::cardkit_streaming::CardkitStreamingRenderer;
use self::outbound_media::{prepare_outbound_local_media, upload_file, upload_image};
use crate::error::AdapterError;
use crate::media_store::MediaStore;
use crate::normalized::{add_code_block_language_hint, normalize_urls};
use crate::IMAdapter;
use async_trait::async_trait;
use chrono::Utc;
use closeclaw_common::identity::IdentityResolver;
use closeclaw_common::processor::{ContentBlock, DslParseResult};
use closeclaw_common::streaming::{DefaultStreamingRenderer, StreamingRenderer};
use closeclaw_common::{
    AdapterError as CommonAdapterError, CardActionEvent, IMPlugin, NormalizedMessage,
    RenderedOutput,
};
use closeclaw_config::identity::ConfigIdentityResolver;
use closeclaw_config::CredentialsProvider;
use closeclaw_debug_log::{DebugLog, LogEvent, LogLevel, TraceContext};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

use super::PlatformEntry;

pub use adapter::FeishuAdapter;
use renderer::build_card;
pub use renderer::build_text;
pub use renderer::should_use_card_for_blocks;

// Re-export adapter internals for test modules.
#[cfg(test)]
pub(crate) use adapter::{
    truncate_to_500, FeishuEvent, FeishuHeader, FeishuMessageEvent, FeishuSender, FeishuSenderId,
};
#[cfg(test)]
pub(crate) use closeclaw_gateway::Message;
#[cfg(test)]
pub(crate) use post_expand::expand_post_content;
#[cfg(test)]
pub(crate) use renderer::extract_card_plain_text;

inventory::submit!(PlatformEntry {
    name: "feishu",
    register: |gw, cfg, ms, mc| {
        let gw = gw.clone();
        let cfg = cfg.to_string();
        Box::pin(async move { register(&gw, &cfg, ms, mc).await })
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

/// Load `{config_dir}/config/media.json`.
///
/// Returns default config when the file is missing or unparseable.
pub(crate) fn load_media_config(config_dir: &str) -> closeclaw_config::MediaConfigData {
    let path = std::path::Path::new(config_dir)
        .join("config")
        .join("media.json");
    match closeclaw_config::MediaConfigData::from_file(&path) {
        Ok(cfg) => {
            info!(
                storage_dir = %cfg.storage_dir,
                "media config loaded from {}",
                path.display()
            );
            cfg
        }
        Err(e) => {
            warn!(
                error = %e,
                path = %path.display(),
                "failed to load media.json — using defaults"
            );
            closeclaw_config::MediaConfigData::default()
        }
    }
}

/// Register the Feishu plugin with the Gateway.
///
/// First checks `{config_dir}/config/platforms.json` for an explicit
/// enable flag.  If the platform is not listed or disabled the plugin
/// is silently not registered.  When enabled, credentials are loaded
/// from `{config_dir}/config/credentials/` (config file first, then
/// `FEISHU_PROFILE` environment variable as fallback).
///
/// Identity mapping is loaded from `{config_dir}/config/accounts.json`
/// (if the file exists).  A missing or empty file results in no
/// mapping — the fallback uses `sender_id` as `account_id`.
pub async fn register(
    gateway: &Arc<closeclaw_gateway::Gateway>,
    config_dir: &str,
    shared_media_store: Option<Arc<MediaStore>>,
    _shared_media_config: Option<closeclaw_config::MediaConfigData>,
) {
    let platforms = load_platforms_config(config_dir);
    if !platforms.is_enabled("feishu") {
        info!("feishu not enabled in platforms.json — skipping");
        return;
    }

    // Load feishu profile from config credentials first, fallback to env var.
    let profile = CredentialsProvider::load_from_dir(
        &std::path::Path::new(config_dir)
            .join("config")
            .join("credentials"),
    )
    .ok()
    .and_then(|creds| creds.feishu_profile().map(|p| p.profile.clone()))
    .or_else(|| std::env::var("FEISHU_PROFILE").ok());
    if let Some(profile) = profile {
        // Use shared MediaStore from daemon if available, otherwise create one.
        let media_store = shared_media_store.unwrap_or_else(|| {
            let media_config = load_media_config(config_dir);
            Arc::new(
                MediaStore::new(&media_config.storage_dir).expect("failed to create media store"),
            )
        });
        let adapter = Arc::new(
            FeishuAdapter::new(profile.clone(), media_store)
                .with_workspace_dir(Some(std::path::PathBuf::from(config_dir))),
        );

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

        // Spawn long-connection event stream (lark-cli event consume).
        let profile_name = profile.clone();
        let (mut pm, event_rx) = process_manager::ProcessManager::new(
            "lark-cli".to_string(),
            vec![
                "event".to_string(),
                "consume".to_string(),
                "--profile".to_string(),
                profile_name,
            ],
        );
        match pm.start().await {
            Ok(()) => {
                process_manager::start_event_stream(gateway, event_rx);
                info!("Feishu long-connection event stream started");
            }
            Err(e) => {
                warn!(
                    error = %e,
                    "failed to start lark-cli event stream — events will not be received"
                );
            }
        }
    } else {
        warn!("feishu enabled in platforms.json but FEISHU_PROFILE not set — skipping");
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
    /// Cardkit streaming renderer for incremental card updates.
    cardkit_streaming: std::sync::Mutex<CardkitStreamingRenderer>,
    /// Default streaming renderer for text line-buffering and block accumulation.
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
            cardkit_streaming: std::sync::Mutex::new(CardkitStreamingRenderer::new()),
            streaming_renderer: std::sync::Mutex::new(DefaultStreamingRenderer::new()),
            debug_log: None,
        }
    }

    /// Create a Feishu plugin with an optional identity resolver.
    pub(crate) fn with_identity_resolver(
        adapter: Arc<FeishuAdapter>,
        identity_resolver: Option<Arc<dyn IdentityResolver>>,
    ) -> Self {
        Self {
            adapter,
            identity_resolver,
            cardkit_streaming: std::sync::Mutex::new(CardkitStreamingRenderer::new()),
            streaming_renderer: std::sync::Mutex::new(DefaultStreamingRenderer::new()),
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

    /// Fallback: extract plain text and media from an interactive card
    /// and send them via text message API and `dispatch_send_media`.
    /// Logs warnings on failure and always returns so the Agent keeps running.
    async fn send_interactive_fallback(
        &self,
        peer_id: &str,
        output: &RenderedOutput,
        reply_ref: Option<&send_helpers::ReplyTarget>,
    ) {
        card_media_fallback::send_interactive_fallback(&self.adapter, peer_id, output, reply_ref)
            .await
    }

    /// Generate a trace_id in the format `{platform}_{timestamp_hex}_{uuid_v4}`.
    ///
    /// - Platform identifier: passed as `platform` parameter
    /// - Timestamp: Unix epoch milliseconds in hex
    /// - Random component: UUID v4 with hyphens removed
    ///
    /// This format allows operators to identify the source platform and approximate
    /// arrival time from the trace_id alone.
    pub(crate) fn generate_trace_id(&self, platform: &str) -> String {
        let timestamp_hex = format!("{:x}", Utc::now().timestamp_millis());
        let uuid_no_hyphens = uuid::Uuid::new_v4().simple().to_string();
        format!("{platform}_{timestamp_hex}_{uuid_no_hyphens}")
    }

    /// Emit a structured debug_log event asynchronously.
    ///
    /// Centralizes the repeated pattern: check debug_log, acquire trace_id,
    /// build event, spawn async send. Callers only supply `event_type` and
    /// `payload`. Skips silently when debug_log is None or trace_id is empty.
    pub(super) fn emit_debug_event(&self, event_type: &str, payload: serde_json::Value) {
        let debug_log = match self.debug_log {
            Some(ref dl) => dl.clone(),
            None => return,
        };
        let trace_id = self
            .adapter
            .last_metadata
            .try_lock()
            .ok()
            .and_then(|m| m.get("trace_id").cloned());
        match trace_id {
            Some(tid) if !tid.is_empty() => {
                let ctx = TraceContext::new_root(tid);
                let event =
                    LogEvent::new(&ctx, None, LogLevel::Info, "feishu", event_type, payload);
                tokio::spawn(async move {
                    debug_log.log(event).await;
                });
            }
            _ => {
                warn!(
                    event_type = %event_type,
                    "emit_debug_event: try_lock failed or trace_id empty — skipping"
                );
            }
        }
    }

    /// Normalize content and apply identity mapping to an inbound message.
    ///
    /// `bot_app_id` is resolved with priority:
    /// 1. `header_app_id` from `last_metadata` (the event header's app_id)
    /// 2. The adapter's own `app_id` (fallback for legacy flows)
    fn normalize_inbound_message(&self, msg: &mut NormalizedMessage) {
        msg.content = normalize_urls(&msg.content);
        msg.content = add_code_block_language_hint(&msg.content);
        if let Some(resolver) = self.identity_resolver() {
            let bot_app_id = match self.adapter.last_metadata.try_lock() {
                Ok(guard) => guard
                    .get("header_app_id")
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .unwrap_or_default(),
                Err(_) => {
                    debug!(
                        platform = %msg.platform,
                        sender_id = %msg.sender_id,
                        "try_lock failed, falling back to empty bot_app_id"
                    );
                    String::new()
                }
            };
            msg.account_id = resolver
                .resolve(&msg.platform, &bot_app_id, &msg.sender_id)
                .unwrap_or(std::mem::take(&mut msg.account_id));
        }
    }

    /// Dispatch a rendered output to the platform send API.
    pub(super) async fn dispatch_send(
        &self,
        peer_id: &str,
        output: &RenderedOutput,
        reply_ref: Option<&send_helpers::ReplyTarget>,
    ) -> Result<(), CommonAdapterError> {
        match output.msg_type.as_str() {
            "text" => {
                let text = output
                    .payload
                    .get("content")
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                if let Err(e) = self
                    .adapter
                    .send_msg(peer_id, "text", text, reply_ref)
                    .await
                {
                    warn!(peer_id = %peer_id, error = %e,
                        "Feishu text send failed — returning Ok(()) per design doc");
                }
                Ok(())
            }
            "interactive" => {
                // Process media elements in the card payload before sending.
                let mut payload = output.payload.clone();
                if let Err(e) = self.process_card_media(&mut payload).await {
                    warn!(peer_id = %peer_id, error = %e,
                        "Failed to process card media — sending as-is");
                }
                let card_json = serde_json::to_string(&payload)
                    .map_err(|e| CommonAdapterError::SendFailed(e.to_string()))?;
                match self
                    .adapter
                    .send_msg(peer_id, "interactive", &card_json, reply_ref)
                    .await
                {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        warn!(peer_id = %peer_id, error = %e,
                            "Feishu interactive card send failed — falling back to text + media");
                        self.send_interactive_fallback(peer_id, output, reply_ref)
                            .await;
                        Ok(())
                    }
                }
            }
            _ => Err(CommonAdapterError::UnsupportedOperation),
        }
    }

    /// Process media elements in a card payload, uploading files to Feishu
    /// and replacing local paths with Feishu image/file keys.
    async fn process_card_media(
        &self,
        payload: &mut serde_json::Value,
    ) -> Result<(), AdapterError> {
        let elements = match payload
            .get_mut("card")
            .and_then(|c| c.get_mut("elements"))
            .and_then(|e| e.as_array_mut())
        {
            Some(e) => e,
            None => return Ok(()),
        };

        for element in elements.iter_mut() {
            let tag = element.get("tag").and_then(|t| t.as_str()).unwrap_or("");
            match tag {
                "img" => self.process_card_img(element).await,
                "media" | "audio" | "file" => self.process_card_media_file(element).await,
                _ => {}
            }
        }
        Ok(())
    }

    /// Process a single `img` element: validate, copy to outbound, upload.
    async fn process_card_img(&self, element: &mut serde_json::Value) {
        let img_key = match element.get("img_key").and_then(|k| k.as_str()) {
            Some(k) => k,
            None => return,
        };
        let media_store = &self.adapter.media_store;
        let workspace_dir = self.adapter.workspace_dir.as_deref();
        let outbound = match prepare_outbound_local_media(img_key, workspace_dir, media_store).await
        {
            Some(p) => p,
            None => return,
        };
        match upload_image(&self.adapter, &outbound).await {
            Ok(key) => {
                if let Some(obj) = element.as_object_mut() {
                    obj.insert("img_key".to_string(), serde_json::Value::String(key));
                }
            }
            Err(e) => warn!(error = %e, "Failed to upload image to Feishu"),
        }
    }

    /// Process a single `media` element: validate, copy to outbound, upload.
    async fn process_card_media_file(&self, element: &mut serde_json::Value) {
        let file_token = match element.get("file_token").and_then(|k| k.as_str()) {
            Some(k) => k,
            None => return,
        };
        let media_store = &self.adapter.media_store;
        let workspace_dir = self.adapter.workspace_dir.as_deref();
        let outbound =
            match prepare_outbound_local_media(file_token, workspace_dir, media_store).await {
                Some(p) => p,
                None => return,
            };
        let filename = outbound
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        match upload_file(&self.adapter, &outbound, &filename).await {
            Ok(key) => {
                if let Some(obj) = element.as_object_mut() {
                    obj.insert("file_token".to_string(), serde_json::Value::String(key));
                }
            }
            Err(e) => warn!(error = %e, "Failed to upload file to Feishu"),
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
        // Generate trace_id at webhook arrival for cross-chain correlation.
        let trace_id = self.generate_trace_id(self.platform());

        let start = Instant::now();
        let mut msg = self
            .adapter
            .parse_inbound(payload)
            .await
            .map_err(convert_to_common_error)?;
        let parse_duration_ms = start.elapsed().as_millis() as u64;

        // Re-insert trace_id after adapter call — adapter's parse_message_event
        // clears last_metadata and repopulates it with chat_name.
        {
            let mut meta = self.adapter.last_metadata.lock().await;
            meta.insert("trace_id".to_string(), trace_id.clone());
        }

        if let Some(ref mut m) = msg {
            self.normalize_inbound_message(m);
        }

        // Emit structured debug_log event for inbound parse.
        let message_type = msg
            .as_ref()
            .map(|m| {
                serde_json::to_value(&m.message_type)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        self.emit_debug_event(
            "inbound.parse",
            serde_json::json!({
                "platform": "feishu",
                "message_type": message_type,
                "parse_duration_ms": parse_duration_ms,
            }),
        );

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
        // Delegate to adapter: handles dedup + deferred discard + debug logging.
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

        let start = Instant::now();
        let (title, elements) = renderer::dispatch_blocks(content_blocks, dsl_result, true);
        let output = build_card(title, elements);
        let render_duration_ms = start.elapsed().as_millis() as u64;

        // Emit structured debug_log event for outbound render.
        self.emit_debug_event(
            "outbound.render",
            serde_json::json!({
                "platform": "feishu",
                "msg_type": output.msg_type,
                "render_duration_ms": render_duration_ms,
            }),
        );

        output
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        peer_id: &str,
        thread_id: Option<&str>,
        reply_ref: Option<&str>,
    ) -> Result<(), CommonAdapterError> {
        // Construct ReplyTarget from thread_id + reply_ref:
        // - Thread (topic): thread_id is Some → Thread { root_id }
        // - Top-level: thread_id is None → reply_ref.map(Message)
        let target = if let Some(tid) = thread_id {
            // Topic conversation: use reply_ref as root_id, fallback to thread_id
            let root_id = reply_ref.unwrap_or(tid).to_string();
            Some(send_helpers::ReplyTarget::Thread { root_id })
        } else {
            // Top-level message: use reply_ref as message_id
            reply_ref.map(|id| send_helpers::ReplyTarget::Message {
                message_id: id.to_string(),
            })
        };

        // During streaming, text messages are routed to cardkit card updates.
        // Non-text (interactive cards) go through normal dispatch for batch mode.
        if output.msg_type == "text" {
            return self
                .send_streaming_text_with_target(output, peer_id, target.as_ref())
                .await;
        }

        self.send_batch_output_with_target(output, peer_id, target.as_ref())
            .await
    }

    async fn shutdown(&self) -> Result<(), CommonAdapterError> {
        // lark-cli manages its own credential lifecycle.
        Ok(())
    }

    fn streaming_renderer(&self) -> Option<&std::sync::Mutex<DefaultStreamingRenderer>> {
        Some(&self.streaming_renderer)
    }

    fn handle_stream_event(
        &self,
        event: closeclaw_common::processor::StreamEvent,
    ) -> closeclaw_common::im_plugin::StreamingOutput {
        // Delegate text processing to the default renderer.
        let out = self
            .streaming_renderer
            .lock()
            .expect("streaming renderer lock poisoned")
            .handle_event(event);
        // Route text_messages to cardkit pending_text for batch card updates.
        if !out.text_messages.is_empty() {
            let mut state = self
                .cardkit_streaming
                .lock()
                .expect("cardkit streaming lock poisoned");
            for msg in &out.text_messages {
                state.state.pending_text.push_str(msg);
            }
        }
        // Return render_blocks (Thinking/Tool) unchanged for Gateway processing.
        out
    }

    fn flush_stream(&self) -> closeclaw_common::im_plugin::StreamingOutput {
        // Delegate flush to the default renderer.
        let out = self
            .streaming_renderer
            .lock()
            .expect("streaming renderer lock poisoned")
            .flush();
        // Route remaining text to cardkit pending_text.
        if !out.text_messages.is_empty() {
            let mut state = self
                .cardkit_streaming
                .lock()
                .expect("cardkit streaming lock poisoned");
            for msg in &out.text_messages {
                state.state.pending_text.push_str(msg);
            }
        }
        // Return render_blocks unchanged for Gateway processing.
        out
    }

    fn check_stream_timeout(&self) -> closeclaw_common::im_plugin::StreamingOutput {
        // Delegate timeout check to the default renderer.
        let out = self
            .streaming_renderer
            .lock()
            .expect("streaming renderer lock poisoned")
            .check_timeout();
        // Route force-emitted text to cardkit pending_text.
        if !out.text_messages.is_empty() {
            let mut state = self
                .cardkit_streaming
                .lock()
                .expect("cardkit streaming lock poisoned");
            for msg in &out.text_messages {
                state.state.pending_text.push_str(msg);
            }
        }
        out
    }

    fn clean_content(&self, raw: &str) -> String {
        cleaner::clean_feishu_content(raw)
    }
}
