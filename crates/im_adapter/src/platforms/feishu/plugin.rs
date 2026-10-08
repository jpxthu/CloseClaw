//! Feishu plugin implementation — `FeishuPlugin` and its `IMPlugin` impl.
//!
//! Unified IM plugin for Feishu messaging platform, wrapping
//! [`FeishuAdapter`](super::FeishuAdapter) (HTTP I/O) behind a single
//! [`IMPlugin`] implementation, plus the startup registration entry
//! ([`register`]) and the compile-time `inventory` entry.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::identity::IdentityResolver;
use closeclaw_common::processor::{ContentBlock, DslParseResult};
use closeclaw_common::streaming::{CodeBlockMode, DefaultStreamingRenderer};
use closeclaw_common::{
    AdapterError as CommonAdapterError, CardActionEvent, IMPlugin, NormalizedMessage,
    RenderedOutput,
};
use closeclaw_debug_log::DebugLog;
use tracing::{info, warn};

use super::cardkit_streaming::CardkitStreamingRenderer;
use super::config::load_platforms_config;
use super::identity;
use super::process_manager;
use super::send_helpers;
use super::{cleaner, FeishuAdapter};
use crate::media_store::MediaStore;
use crate::platforms::{MediaConfigSnapshot, PlatformEntry};
use crate::IMAdapter;

inventory::submit!(PlatformEntry {
    name: "feishu",
    register: |host, cfg, ms, mc, ir, fp| {
        let host = host.clone();
        let cfg = cfg.to_string();
        Box::pin(async move { register(&host, &cfg, ms, mc, ir, fp).await })
    },
});

/// Resolve the effective Feishu profile: the injected profile wins; the
/// environment value (read lazily through `env_profile` by the caller) is
/// the fallback.
///
/// Extracted as a seam so tests can exercise both branches without mutating
/// the process environment (docs/developer/STANDARDS.md §7 forbids env
/// mutation outside `load_env_file`).
pub(super) fn resolve_feishu_profile(
    injected: Option<String>,
    env_profile: impl FnOnce() -> Option<String>,
) -> Option<String> {
    injected.or_else(env_profile)
}

/// Register the Feishu plugin with the host (composition-root ports).
///
/// First checks `{config_dir}/config/platforms.json` for an explicit
/// enable flag.  If the platform is not listed or disabled the plugin
/// is silently not registered.  When enabled, the profile is taken from
/// the injected `feishu_profile` first, then the `FEISHU_PROFILE`
/// environment variable as fallback.
///
/// The identity resolver is injected by the composition root (loaded
/// from `{config_dir}/config/accounts.json` there).  A missing /
/// empty mapping set results in no resolver — the fallback uses
/// `sender_id` as `account_id`.
pub async fn register(
    host: &crate::ports::GatewayHost,
    config_dir: &str,
    shared_media_store: Option<Arc<MediaStore>>,
    media_config: Option<MediaConfigSnapshot>,
    identity_resolver: Option<Arc<dyn IdentityResolver>>,
    feishu_profile: Option<String>,
) {
    let platforms = load_platforms_config(config_dir);
    if !platforms.is_enabled("feishu") {
        info!("feishu not enabled in platforms.json — skipping");
        return;
    }

    // Injected profile first, FEISHU_PROFILE environment variable as fallback.
    let profile = resolve_feishu_profile(feishu_profile, || std::env::var("FEISHU_PROFILE").ok());
    if let Some(profile) = profile {
        // Use shared MediaStore from daemon if available, otherwise create one.
        let media_store = shared_media_store.unwrap_or_else(|| {
            let snapshot = media_config.clone().unwrap_or_default();
            Arc::new(
                MediaStore::new(&snapshot.storage_dir.to_string_lossy())
                    .expect("failed to create media store"),
            )
        });
        let adapter = Arc::new(
            FeishuAdapter::new(profile.clone(), media_store)
                .with_workspace_dir(Some(std::path::PathBuf::from(config_dir))),
        );

        let mut plugin = FeishuPlugin::with_identity_resolver(adapter, identity_resolver);

        // Injected DebugLog (configured on the host, if any).
        if let Some(debug_log) = host.debug_log.clone() {
            plugin.set_debug_log(Arc::new(debug_log));
        }

        let plugin: Arc<dyn IMPlugin> = Arc::new(plugin);
        host.registrar.register_plugin(plugin).await;
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
                process_manager::start_event_stream(host.enqueuer.clone(), event_rx);
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
        self.parse_inbound_payload(payload).await
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
        self.render_blocks(content_blocks)
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
