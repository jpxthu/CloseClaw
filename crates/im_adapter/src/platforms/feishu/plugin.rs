//! Feishu plugin implementation — `FeishuPlugin` and its `IMPlugin` impl.
//!
//! Unified IM plugin for Feishu messaging platform, wrapping
//! [`FeishuAdapter`](super::FeishuAdapter) (HTTP I/O) behind a single
//! [`IMPlugin`] implementation, plus the startup registration entry
//! ([`register`]) and the compile-time `inventory` entry.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use closeclaw_common::identity::IdentityResolver;
use closeclaw_common::processor::{ContentBlock, DslParseResult};
use closeclaw_common::streaming::{CodeBlockMode, DefaultStreamingRenderer};
use closeclaw_common::{
    AdapterError as CommonAdapterError, CardActionEvent, IMPlugin, NormalizedMessage,
    RenderedOutput,
};
use closeclaw_config::CredentialsProvider;
use closeclaw_debug_log::DebugLog;
use tracing::{info, warn};

use super::cardkit_streaming::CardkitStreamingRenderer;
use super::config::{load_media_config, load_platforms_config};
use super::identity;
use super::process_manager;
use super::renderer::{self, build_card};
use super::send_helpers;
use super::{build_text, cleaner, should_use_card_for_blocks, FeishuAdapter};
use crate::media_store::MediaStore;
use crate::platforms::PlatformEntry;
use crate::IMAdapter;

inventory::submit!(PlatformEntry {
    name: "feishu",
    register: |gw, cfg, ms, mc| {
        let gw = gw.clone();
        let cfg = cfg.to_string();
        Box::pin(async move { register(&gw, &cfg, ms, mc).await })
    },
});

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
            identity::load_identity_resolver(config_dir);

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

/// Unified IM plugin for Feishu.
pub struct FeishuPlugin {
    pub(super) adapter: Arc<FeishuAdapter>,
    pub(super) identity_resolver: Option<Arc<dyn IdentityResolver>>,
    /// Cardkit streaming renderer for incremental card updates.
    pub(super) cardkit_streaming: std::sync::Mutex<CardkitStreamingRenderer>,
    /// Default streaming renderer for text line-buffering and block accumulation.
    streaming_renderer: std::sync::Mutex<DefaultStreamingRenderer>,
    /// Debug log framework instance for structured event logging.
    pub(super) debug_log: Option<Arc<DebugLog>>,
}

impl FeishuPlugin {
    #[allow(dead_code)]
    pub(crate) fn new(adapter: Arc<FeishuAdapter>) -> Self {
        Self {
            adapter,
            identity_resolver: None,
            cardkit_streaming: std::sync::Mutex::new(CardkitStreamingRenderer::new()),
            streaming_renderer: std::sync::Mutex::new(
                DefaultStreamingRenderer::new().with_code_block_mode(CodeBlockMode::WholeBlock),
            ),
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

    /// Return the current cardkit pending text (test-only).
    #[cfg(test)]
    pub(crate) fn cardkit_pending_text(&self) -> String {
        self.cardkit_streaming
            .lock()
            .expect("cardkit lock poisoned")
            .state
            .pending_text
            .clone()
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
            .map_err(identity::convert_to_common_error)?;
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
            .map_err(identity::convert_to_common_error)
    }

    async fn validate_signature(&self, signature: &str, payload: &[u8]) -> bool {
        self.adapter.validate_signature(signature, payload).await
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        if content_blocks.is_empty() {
            return build_text("");
        }

        if content_blocks.len() == 1 {
            if let ContentBlock::Text(text) = &content_blocks[0] {
                if !renderer::should_use_card(text, false) {
                    return build_text(text.trim());
                }
            }
        }

        if !should_use_card_for_blocks(content_blocks, false) {
            return build_text("");
        }

        let start = Instant::now();
        let (title, elements) = renderer::dispatch_blocks(content_blocks, None, true);
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

    fn clean_content(&self, raw: &str) -> String {
        cleaner::clean_feishu_content(raw)
    }
}
