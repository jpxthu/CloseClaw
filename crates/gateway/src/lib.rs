//! Gateway - IM protocol adapters, message routing, authentication
//!
//! Central hub that connects IM platforms (Feishu, Discord, etc.) to agents.
pub mod approval;
#[cfg(test)]
pub mod approval_tests;
pub mod card_action;
mod debug_log_emitter;
#[cfg(test)]
mod debug_log_emitter_tests;
#[cfg(test)]
pub mod debug_log_tests;
pub(crate) mod health_check_builders;
#[cfg(test)]
mod health_check_builders_tests;
pub(crate) mod idle_verify_hook;
#[cfg(test)]
mod im_adapter;
pub mod inbound_queue;
#[cfg(test)]
mod inbound_queue_ack_tests;
#[cfg(test)]
mod inbound_queue_arrived_tests;
#[cfg(test)]
mod inbound_queue_test_utils;
#[cfg(test)]
mod inbound_queue_tests;
pub(crate) mod inbound_wal;
#[cfg(test)]
mod inbound_wal_tests;
pub mod llm_caller_impl;
mod media_routing;
mod memory;
pub mod message;
mod message_routing;
pub mod outbound;
#[cfg(test)]
mod outbound_batch_failure_tests;
#[cfg(test)]
mod outbound_checkpoint_last_message_at_tests;
#[cfg(test)]
mod outbound_checkpoint_timing_tests;
#[cfg(test)]
mod outbound_dsl_passthrough_tests;
#[cfg(test)]
mod outbound_fallback_tests;
mod outbound_helpers;
#[cfg(test)]
mod outbound_helpers_tests;
pub mod outbound_middleware;
#[cfg(test)]
mod outbound_streaming_checkpoint_tests;
#[cfg(test)]
mod outbound_streaming_dsl_checkpoint_tests;
#[cfg(test)]
mod outbound_tests;
mod processor_registry_builder;
#[cfg(test)]
mod receiving_transition_tests;
mod resolve_session;
pub mod session_handler;
mod session_handler_announce;
mod session_handler_compact;
mod session_handler_dispatch;
mod session_handler_streaming;
pub mod session_manager;
mod shutdown_card;
pub mod shutdown_handle;
pub mod slash_executor_helpers;
#[cfg(test)]
mod slash_executor_system_append_tests;
#[cfg(test)]
mod slash_executor_tests;
pub mod slash_permission;
pub mod slash_permission_handlers;
#[cfg(test)]
mod slash_permission_outbound_tests;
#[cfg(test)]
mod slash_permission_tests;
#[cfg(test)]
mod step1_4_e2e_tests;
#[cfg(test)]
mod step1_5c_tests;
#[cfg(test)]
mod streaming_pipeline_tests;
#[cfg(test)]
mod streaming_preflight_tests;
pub mod sweeper;
mod sweeper_active_query_tests;
#[cfg(test)]
mod sweeper_tests;
#[cfg(test)]
pub mod tests_checkpoint;
#[cfg(feature = "full-tests")]
mod tests_plugin;

#[cfg(feature = "full-tests")]
mod slash_permission_routing_tests;
#[cfg(test)]
mod tests_slash_dispatcher_routing;
#[cfg(feature = "full-tests")]
mod tests_slash_permission;
#[cfg(feature = "full-tests")]
mod tests_slash_permission_integration;
pub mod types;
pub mod workflow_owner;
use inbound_queue::InboundDebugCtx;
pub use outbound::OutboundMeta;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

pub use closeclaw_common::processor::ProcessorChain;
use closeclaw_common::processor::{ContentBlock, ProcessedMessage};
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_common::slash_router::SlashRouter;
use closeclaw_common::MediaStoreAccess;
use closeclaw_config::MediaConfigData;
use closeclaw_debug_log::{DebugLog, LogLevel};
use closeclaw_llm::ProviderModelKnowledge;
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_session::checkpoint_manager::CheckpointManager;
use closeclaw_session::persistence::PersistenceService;
pub use inbound_queue::{InboundQueueFull, InboundQueueHandle, InboundRequest};
pub(crate) use rebuild_stash::RebuildStash;
pub use types::*;

pub use session_handler::{HandleResult, SessionMessageHandler};
pub use session_manager::{SessionManager, SpawnController};
pub use shutdown_handle::ShutdownHandle;

/// Routes messages between IM plugins and agents.
pub struct Gateway {
    config: GatewayConfig,
    plugins: RwLock<HashMap<String, Arc<dyn closeclaw_common::IMPlugin>>>,
    session_manager: Arc<SessionManager>,
    processor_registry: std::sync::RwLock<Option<Arc<dyn ProcessorChain>>>,
    checkpoint_manager: std::sync::RwLock<Option<Arc<CheckpointManager<dyn PersistenceService>>>>,
    session_handler: std::sync::OnceLock<Arc<SessionMessageHandler>>,
    approval_flow: RwLock<Option<Arc<tokio::sync::Mutex<ApprovalFlow>>>>,
    slash_dispatcher: RwLock<Option<Arc<dyn SlashRouter>>>,
    permission_engine: RwLock<Option<Arc<tokio::sync::RwLock<PermissionEngine>>>>,
    inbound_tx: std::sync::Mutex<Option<mpsc::Sender<inbound_queue::QueuedInbound>>>,
    /// Back-pointer to owning `Arc<Gateway>` for streaming dispatch.
    self_ref: std::sync::Mutex<Option<Arc<Gateway>>>,
    /// Shutdown handle for busy-count tracking during drain.
    shutdown_handle: std::sync::Mutex<Option<Arc<ShutdownHandle>>>,
    outbound_middlewares: std::sync::RwLock<Vec<Arc<dyn closeclaw_common::OutboundMiddleware>>>,
    config_dir: RwLock<Option<std::path::PathBuf>>,
    metrics_emitter: std::sync::RwLock<Option<Arc<dyn closeclaw_common::MetricsEmitter>>>,
    debug_log: std::sync::RwLock<Option<DebugLog>>,
    inbound_wal: std::sync::Mutex<Option<Arc<inbound_wal::InboundWal>>>,
    pub(crate) rebuild_stash: Arc<RebuildStash>,
    media_store: std::sync::Mutex<Option<Arc<dyn MediaStoreAccess>>>,
    media_config: std::sync::RwLock<MediaConfigData>,
}

/// Result of inbound pre-validation gates.
pub(crate) use media_routing::InboundValidation;

impl Gateway {
    /// Create a new Gateway with the given config and a shared SessionManager.
    pub fn new(config: GatewayConfig, session_manager: Arc<SessionManager>) -> Self {
        let registry = build_processor_registry(&config);
        let gw = Self {
            config,
            plugins: RwLock::new(HashMap::new()),
            session_manager,
            processor_registry: std::sync::RwLock::new(Some(Arc::new(registry))),
            checkpoint_manager: std::sync::RwLock::new(None),
            session_handler: std::sync::OnceLock::new(),
            approval_flow: RwLock::new(None),
            slash_dispatcher: RwLock::new(None),
            permission_engine: RwLock::new(None),
            inbound_tx: std::sync::Mutex::new(None),
            self_ref: std::sync::Mutex::new(None),
            shutdown_handle: std::sync::Mutex::new(None),
            outbound_middlewares: std::sync::RwLock::new(Vec::new()),
            config_dir: RwLock::new(None),
            metrics_emitter: std::sync::RwLock::new(None),
            debug_log: std::sync::RwLock::new(None),
            inbound_wal: std::sync::Mutex::new(None),
            rebuild_stash: Arc::new(RebuildStash::new()),
            media_store: std::sync::Mutex::new(None),
            media_config: std::sync::RwLock::new(MediaConfigData::default()),
        };
        register_default_middlewares(&gw, &gw.config);
        gw
    }

    /// Create a new Gateway with the given config, SessionManager and ProcessorRegistry.
    pub fn with_processor_registry(
        config: GatewayConfig,
        session_manager: Arc<SessionManager>,
        registry: Arc<dyn ProcessorChain>,
    ) -> Self {
        let gw = Self {
            config,
            plugins: RwLock::new(HashMap::new()),
            session_manager,
            processor_registry: std::sync::RwLock::new(Some(registry)),
            checkpoint_manager: std::sync::RwLock::new(None),
            session_handler: std::sync::OnceLock::new(),
            approval_flow: RwLock::new(None),
            slash_dispatcher: RwLock::new(None),
            permission_engine: RwLock::new(None),
            inbound_tx: std::sync::Mutex::new(None),
            self_ref: std::sync::Mutex::new(None),
            shutdown_handle: std::sync::Mutex::new(None),
            outbound_middlewares: std::sync::RwLock::new(Vec::new()),
            config_dir: RwLock::new(None),
            metrics_emitter: std::sync::RwLock::new(None),
            debug_log: std::sync::RwLock::new(None),
            inbound_wal: std::sync::Mutex::new(None),
            rebuild_stash: Arc::new(RebuildStash::new()),
            media_store: std::sync::Mutex::new(None),
            media_config: std::sync::RwLock::new(MediaConfigData::default()),
        };
        register_default_middlewares(&gw, &gw.config);
        gw
    }

    /// Enter or exit rebuild mode on the shared stash buffer.
    pub fn set_rebuild_mode(&self, enabled: bool) {
        self.rebuild_stash.set_rebuild_mode(enabled);
    }

    /// Drain all stashed inbound requests in FIFO order.
    pub fn take_rebuild_stashed(&self) -> Vec<InboundRequest> {
        self.rebuild_stash.take_stashed()
    }
    /// Push a single request back into the rebuild stash buffer.
    pub fn push_rebuild_stashed(&self, request: InboundRequest) {
        self.rebuild_stash.push(request);
    }

    /// Set config directory for permission rule persistence.
    pub async fn set_config_dir(&self, path: std::path::PathBuf) {
        *self.config_dir.write().await = Some(path);
    }

    pub async fn get_config_dir(&self) -> Option<std::path::PathBuf> {
        self.config_dir.read().await.clone()
    }

    /// Configure a CheckpointManager for session snapshot persistence.
    pub fn with_checkpoint_manager(
        self,
        cm: Arc<CheckpointManager<dyn PersistenceService>>,
    ) -> Self {
        *self.checkpoint_manager.write().unwrap() = Some(cm);
        self
    }

    /// Set the CheckpointManager for session snapshot persistence.
    ///
    /// Runtime setter (non-builder) so an already-constructed `Gateway`
    /// can receive the shared [`CheckpointManager`] — e.g. during a
    /// config-triggered gateway restart.
    pub fn set_checkpoint_manager(&self, cm: Arc<CheckpointManager<dyn PersistenceService>>) {
        *self.checkpoint_manager.write().unwrap() = Some(cm);
    }

    /// Check if a CheckpointManager is currently set.
    pub fn has_checkpoint_manager(&self) -> bool {
        self.checkpoint_manager.read().unwrap().is_some()
    }

    /// Configure a SessionMessageHandler for busy/pending LLM session management.
    /// When a handler is installed, inbound messages are routed through the
    /// busy/pending state machine. When `None` (default), Gateway behaves as before.
    pub fn with_session_handler(self, handler: Arc<SessionMessageHandler>) -> Self {
        let _ = self.session_handler.set(handler);
        self
    }

    /// Set the session handler (ARC-safe setter).
    ///
    /// Unlike [`with_session_handler`](Self::with_session_handler) which
    /// consumes `self` (used during Gateway construction), this method
    /// takes `&self` so it can be called on an already-wrapped
    /// `Arc<Gateway>`.
    pub fn set_session_handler(&self, handler: Arc<SessionMessageHandler>) {
        let _ = self.session_handler.set(handler);
    }

    /// Wire the back-reference to the owning `Arc<Gateway>` for streaming dispatch.
    pub fn set_self_ref(&self, arc: Arc<Gateway>) {
        if let Ok(mut slot) = self.self_ref.lock() {
            *slot = Some(arc);
        }
    }

    /// Set the shutdown handle for busy-count tracking during drain.
    pub fn set_shutdown_handle(&self, handle: Arc<ShutdownHandle>) {
        if let Ok(mut slot) = self.shutdown_handle.lock() {
            *slot = Some(handle);
        }
    }

    /// Register an outbound middleware.
    ///
    /// Middlewares run in insertion order between [`IMPlugin::render`]
    /// and [`IMPlugin::send`] on every outbound message.
    pub fn add_outbound_middleware(&self, mw: Arc<dyn closeclaw_common::OutboundMiddleware>) {
        if let Ok(mut mws) = self.outbound_middlewares.write() {
            mws.push(mw);
        }
    }

    /// Return the current outbound middleware chain (snapshot).
    pub(crate) async fn get_outbound_middlewares(
        &self,
    ) -> Vec<Arc<dyn closeclaw_common::OutboundMiddleware>> {
        self.outbound_middlewares.read().unwrap().clone()
    }

    /// Set the metrics emitter for operational metrics.
    pub async fn set_metrics_emitter(&self, emitter: Arc<dyn closeclaw_common::MetricsEmitter>) {
        if let Ok(mut slot) = self.metrics_emitter.write() {
            *slot = Some(emitter);
        }
    }

    /// Inject a [`DebugLog`] instance for structured event logging.
    pub async fn set_debug_log(&self, debug_log: DebugLog) {
        if let Ok(mut slot) = self.debug_log.write() {
            *slot = Some(debug_log);
        }
    }

    /// Inject a [`MediaStoreAccess`] for file persistence.
    pub fn set_media_store(&self, store: Arc<dyn MediaStoreAccess>) {
        if let Ok(mut s) = self.media_store.lock() {
            *s = Some(store);
        }
    }
    /// Get the current media store, if configured.
    pub fn get_media_store(&self) -> Option<Arc<dyn MediaStoreAccess>> {
        self.media_store.lock().ok().and_then(|s| s.clone())
    }
    /// Update the media configuration.
    pub fn set_media_config(&self, config: MediaConfigData) {
        if let Ok(mut g) = self.media_config.write() {
            *g = config;
        }
    }
    /// Get the current image content threshold in bytes.
    pub fn image_content_threshold(&self) -> u64 {
        self.media_config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .image_content_threshold_bytes
    }

    /// Get a clone of the current [`DebugLog`] instance, if configured.
    pub fn get_debug_log(&self) -> Option<DebugLog> {
        self.debug_log
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Start the inbound bounded queue.
    ///
    /// Creates a bounded mpsc channel with capacity from
    /// [`GatewayConfig::inbound_queue_capacity`], stores the sender
    /// for later use by [`Self::enqueue_inbound`], and spawns a
    /// consumer task that drains messages through the processor chain
    /// and inbound handler.
    ///
    /// Returns an [`InboundQueueHandle`] that callers can use to
    /// enqueue inbound requests.
    pub fn start_inbound_queue(self: &Arc<Self>) -> inbound_queue::InboundQueueHandle {
        let capacity = self.config.inbound_queue_capacity;
        let (tx, rx) = tokio::sync::mpsc::channel(capacity);
        if let Ok(mut slot) = self.inbound_tx.lock() {
            *slot = Some(tx.clone());
        }
        self.init_inbound_wal();
        self.replay_wal_entries(&tx);
        let wal = self
            .inbound_wal
            .lock()
            .ok()
            .and_then(|s| s.as_ref().cloned());
        inbound_queue::start_inbound_consumer(rx, Arc::clone(self), capacity, wal);
        inbound_queue::InboundQueueHandle::new(tx)
    }

    /// Open the inbound WAL directory if configured.
    fn init_inbound_wal(&self) {
        if let Some(ref wal_dir) = self.config.inbound_wal_dir {
            match inbound_wal::InboundWal::open(wal_dir) {
                Ok(wal) => {
                    if let Ok(mut slot) = self.inbound_wal.lock() {
                        *slot = Some(Arc::new(wal));
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        wal_dir = %wal_dir.display(),
                        error = %e,
                        "failed to open inbound WAL — falling back to in-memory queue"
                    );
                }
            }
        }
    }

    /// Replay unfinished WAL entries into the inbound channel.
    ///
    /// Loads all WAL entries, deduplicates by trace_id, and sends each
    /// unfinished entry into the provided channel.
    fn replay_wal_entries(&self, tx: &mpsc::Sender<inbound_queue::QueuedInbound>) {
        let wal_guard = match self.inbound_wal.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        let Some(ref wal) = *wal_guard else {
            return;
        };
        let entries = match wal.load_all() {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "WAL replay: failed to load entries");
                return;
            }
        };
        let mut seen = std::collections::HashSet::new();
        let mut replayed = 0u64;
        for entry in entries {
            if !seen.insert(entry.trace_id.clone()) {
                continue;
            }
            if self.try_replay_entry(entry, tx).is_ok() {
                replayed += 1;
            }
        }
        if replayed > 0 {
            tracing::info!(count = replayed, "WAL replay complete");
        }
    }

    /// Attempt to replay a single WAL entry into the channel.
    ///
    /// Decodes the payload, constructs a [`QueuedInbound`], sends it,
    /// and emits a `queue.replayed` debug event on success.
    fn try_replay_entry(
        &self,
        entry: inbound_wal::InboundWalEntry,
        tx: &mpsc::Sender<inbound_queue::QueuedInbound>,
    ) -> Result<(), ()> {
        let payload = entry.decoded_payload().map_err(|e| {
            tracing::warn!(
                trace_id = %entry.trace_id,
                error = %e,
                "WAL replay: failed to decode payload — skipping"
            );
        })?;
        let req = inbound_queue::InboundRequest {
            platform: entry.platform,
            raw_payload: payload,
            peer_id: entry.peer_id,
            trace_id: entry.trace_id,
            span_id: None,
        };
        let queued = inbound_queue::QueuedInbound { request: req };
        let trace_id = queued.request.trace_id.clone();
        let platform = queued.request.platform.clone();
        let peer_id = queued.request.peer_id.clone();
        if tx.try_send(queued).is_err() {
            tracing::warn!(trace_id = %trace_id, "WAL replay: queue full — dropping");
            return Err(());
        }
        self.emit_replayed_event(&trace_id, &platform, &peer_id);
        Ok(())
    }

    /// Emit a `queue.replayed` debug event for a successfully replayed entry.
    fn emit_replayed_event(&self, trace_id: &str, platform: &str, peer_id: &str) {
        let guard = self.debug_log.read().unwrap_or_else(|e| e.into_inner());
        debug_log_emitter::emit_debug_event(debug_log_emitter::EmitEventParams {
            ctx: debug_log_emitter::DebugLogContext::new(guard.as_ref(), trace_id, None),
            level: closeclaw_debug_log::LogLevel::Info,
            source_module: "gateway",
            event_type: "queue.replayed",
            payload: serde_json::json!({
                "platform": platform,
                "peer_id": peer_id,
            }),
            parent: None,
        });
    }

    /// Enqueue an inbound request into the bounded queue.
    ///
    /// When the queue is full, a busy reply is sent via the IM plugin
    /// and `Err(InboundQueueFull)` is returned so the caller can decide
    /// the HTTP response status.
    /// If the queue has not been started, the message is processed
    /// directly (bypass mode) and `Ok(())` is returned.
    pub async fn enqueue_inbound(
        &self,
        request: inbound_queue::InboundRequest,
    ) -> Result<(), inbound_queue::InboundQueueFull> {
        inbound_queue::enqueue_inbound(self, request).await
    }

    /// Get a clone of the shutdown handle, if set.
    pub(crate) fn get_shutdown_handle(&self) -> Option<Arc<ShutdownHandle>> {
        self.shutdown_handle.lock().ok().and_then(|s| s.clone())
    }

    pub async fn has_slash_dispatcher(&self) -> bool {
        self.slash_dispatcher.read().await.is_some()
    }

    pub async fn has_session_handler(&self) -> bool {
        self.session_handler.get().is_some()
    }

    /// Returns a reference to the model knowledge base, if the session handler is set.
    pub fn model_knowledge(&self) -> Option<&ProviderModelKnowledge> {
        self.session_handler.get().and_then(|h| h.model_knowledge())
    }

    pub fn config_name(&self) -> &str {
        &self.config.name
    }

    /// Returns `(inbound_count, outbound_count)` for the processor registry.
    pub fn processor_registry_len(&self) -> (usize, usize) {
        let guard = self.processor_registry.read().unwrap();
        match guard.as_ref() {
            Some(registry) => (registry.inbound_len(), registry.outbound_len()),
            None => (0, 0),
        }
    }

    /// Emit a gateway debug event with the given context.
    ///
    /// Convenience helper that reads the debug-log guard and emits the
    /// event in one call, reducing boilerplate in callers.
    pub(crate) fn emit_gateway_event(
        &self,
        trace_id: &str,
        session_key: Option<&str>,
        event_type: &str,
        payload: serde_json::Value,
        parent: Option<&closeclaw_debug_log::TraceContext>,
    ) {
        let guard = self.debug_log.read().unwrap_or_else(|e| e.into_inner());
        debug_log_emitter::emit_debug_event(debug_log_emitter::EmitEventParams {
            ctx: debug_log_emitter::DebugLogContext::new(guard.as_ref(), trace_id, session_key),
            level: LogLevel::Info,
            source_module: "gateway",
            event_type,
            payload,
            parent,
        });
    }

    /// Send a simplified rejection reply and return `None`.
    ///
    /// Shared helper for pre-session-resolution gates (non-text,
    /// unavailable_media, max_message_size, session routing failure).
    pub(crate) async fn reject_with_reply(
        &self,
        peer_id: &str,
        channel: &str,
        msg: &str,
    ) -> Option<HandleResult> {
        if !peer_id.is_empty() {
            if let Err(e) =
                crate::outbound_helpers::send_simplified_with_timeout(self, peer_id, channel, msg)
                    .await
            {
                tracing::warn!(error = %e, "failed to send rejection reply");
            }
        }
        None
    }

    /// Validate inbound message using media routing rules.
    async fn validate_inbound(
        &self,
        processed: &ProcessedMessage,
        peer_id: &str,
        channel: &str,
    ) -> InboundValidation {
        media_routing::validate_inbound(self, processed, peer_id, channel).await
    }

    /// Check session gates (restore, shutdown, stopped, new user, approval).
    ///
    /// Returns `Some(HandleResult)` when the message should be rejected
    /// or handled by a gate; `None` means all gates passed.
    async fn check_session_gates(
        &self,
        session_id: &str,
        content: &str,
        sender_id: Option<&str>,
        peer_id: &str,
        channel: &str,
    ) -> Option<HandleResult> {
        // Restore notification for archived sessions (design doc).
        if let Some((chat_id, custom_msg)) = self
            .session_manager
            .take_restore_notification(session_id)
            .await
        {
            let msg = custom_msg.as_deref().unwrap_or("正在恢复会话...");
            self.send_system_notification(&chat_id, channel, msg).await;
        }
        // Shutdown gate: reject new operations.
        if let Some(sh) = self.get_shutdown_handle() {
            if sh.is_shutting_down() {
                tracing::warn!(session_id = %session_id, "rejecting inbound: shutting down");
                return Some(HandleResult::MessageQueued("".to_string()));
            }
        }
        // Session stopped gate: reject new messages (design doc).
        if let Some(cs) = self
            .session_manager
            .get_conversation_session(session_id)
            .await
        {
            if cs.read().await.is_stopped() {
                tracing::warn!(session_id = %session_id, "rejecting inbound: session stopped");
                return Some(HandleResult::MessageQueued("".to_string()));
            }
        }
        // New user auto-registration (design doc).
        if let Some(sender) = sender_id {
            if let Some(result) = self.check_new_user_registration(sender, channel).await {
                return Some(result);
            }
        }
        // Approval command interception.
        if let Some(result) = self
            .try_handle_approval_command(session_id, content, sender_id, peer_id, channel)
            .await
        {
            return Some(result);
        }
        None
    }

    /// Dispatch a message to the session handler (streaming or non-streaming).
    #[allow(clippy::too_many_arguments)]
    async fn dispatch_to_handler(
        &self,
        processed: &ProcessedMessage,
        session_id: &str,
        content: String,
        blocks: Option<Vec<ContentBlock>>,
        sender_id: Option<&str>,
        peer_id: &str,
        channel: &str,
        dbg: &InboundDebugCtx<'_>,
    ) -> Option<HandleResult> {
        let handler = self.session_handler.get().cloned()?;
        let gw_arc = self
            .self_ref
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(Arc::clone));
        if let (Some(gw), Some(plugin)) = (gw_arc, self.get_plugin(channel).await) {
            let chat_name = processed
                .metadata
                .get("chat_name")
                .cloned()
                .unwrap_or_default();
            let meta = crate::session_handler::MessageMetadata {
                sender_id: sender_id.unwrap_or("").to_string(),
                channel: channel.to_string(),
                timestamp: chrono::Utc::now().timestamp(),
                chat_name,
                trace_id: dbg.trace_id.map(|s: &str| s.to_string()),
                session_key: processed.metadata.get("session_key").cloned(),
                span_id: processed.metadata.get("span_id").cloned(),
            };
            let result = handler
                .handle_message_with_gateway(session_id, content, blocks, meta, &gw, &plugin)
                .await;
            self.maybe_send_notification(&result, peer_id, channel)
                .await;
            return Some(result);
        }
        let result = handler.handle_message(session_id, content).await;
        self.maybe_send_notification(&result, peer_id, channel)
            .await;
        Some(result)
    }

    /// Route and dispatch an inbound message to slash or LLM handler.
    #[allow(clippy::too_many_arguments)]
    async fn route_and_dispatch(
        &self,
        processed: &ProcessedMessage,
        session_id: &str,
        content: String,
        blocks: Option<Vec<ContentBlock>>,
        sender_id: Option<&str>,
        peer_id: &str,
        channel: &str,
        dbg: &InboundDebugCtx<'_>,
    ) -> Option<HandleResult> {
        let is_slash = content.starts_with('/');
        if let Some(tid) = dbg.trace_id {
            self.emit_gateway_event(
                tid,
                dbg.session_key,
                "route.decision",
                serde_json::json!({
                    "session_id": session_id,
                    "decision": if is_slash { "slash" } else { "normal" },
                    "content_prefix": content.chars().take(16).collect::<String>(),
                }),
                dbg.root_ctx.as_ref(),
            );
        }
        if is_slash {
            if let Some(result) = self
                .try_handle_approval_command(session_id, &content, sender_id, peer_id, channel)
                .await
            {
                return Some(result);
            }
            if let Some(result) = self
                .dispatch_slash(session_id, &content, sender_id, channel, Some(peer_id))
                .await
            {
                return Some(result);
            }
        }
        if let Some(result) = self
            .try_handle_workflow_owner_response(session_id, &content, sender_id)
            .await
        {
            return Some(result);
        }
        self.dispatch_to_handler(
            processed, session_id, content, blocks, sender_id, peer_id, channel, dbg,
        )
        .await
    }

    /// Handle an inbound message through the busy/pending state machine.
    pub async fn handle_inbound_message(
        &self,
        processed: ProcessedMessage,
        sender_id: Option<&str>,
        channel: &str,
    ) -> Option<HandleResult> {
        let dbg = inbound_queue::prepare_inbound_debug(self, &processed, sender_id, channel);
        let peer_id = processed
            .metadata
            .get("peer_id")
            .map(|s| s.as_str())
            .unwrap_or("");
        match self.validate_inbound(&processed, peer_id, channel).await {
            InboundValidation::Continue => {}
            InboundValidation::Reject(result) => return Some(result),
            InboundValidation::RejectSilently => return None,
        }
        let ms = self.get_media_store();
        let t = self.image_content_threshold();
        let ms = ms.as_deref();
        let content = media_routing::build_context_content(&processed, ms, t).await;
        let b = media_routing::build_context_content_blocks(&processed, ms, t).await;
        let blocks =
            Some(b).filter(|v| v.iter().any(|bl| matches!(bl, ContentBlock::Image { .. })));
        let session_id = match inbound_queue::resolve_session_with_log(
            self,
            &processed,
            channel,
            peer_id,
            dbg.trace_id,
            dbg.session_key,
            &dbg.root_ctx,
        )
        .await
        {
            Ok(id) => id,
            Err(Some(result)) => return Some(result),
            Err(None) => return None,
        };
        if let Some(result) = self
            .check_session_gates(&session_id, &content, sender_id, peer_id, channel)
            .await
        {
            return Some(result);
        }
        self.route_and_dispatch(
            &processed,
            &session_id,
            content,
            blocks,
            sender_id,
            peer_id,
            channel,
            &dbg,
        )
        .await
    }

    /// If `result` carries a user-facing message (`MessageQueued` or `Error`),
    /// send it as a system notification.  No-op for other variants or empty peer_id.
    async fn maybe_send_notification(&self, result: &HandleResult, peer_id: &str, channel: &str) {
        if peer_id.is_empty() {
            return;
        }
        let text = match result {
            HandleResult::MessageQueued(t) => t,
            HandleResult::Error(t) => t,
            _ => return,
        };
        self.send_system_notification(peer_id, channel, text).await;
    }

    /// Configure the persistence storage backend (proxied to SessionManager).
    pub async fn set_storage(&self, storage: Arc<dyn PersistenceService>) {
        self.session_manager.set_storage(storage).await;
    }

    /// Flush all active sessions to persistence (proxied to SessionManager).
    pub async fn flush_all_sessions(
        &self,
        mode: ShutdownMode,
    ) -> Result<usize, closeclaw_session::persistence::PersistenceError> {
        self.session_manager.flush_all(mode).await
    }

    /// Force a WAL checkpoint via the persistence backend (proxied to
    /// SessionManager).  Call after `flush_all_sessions` in Phase 4.
    pub async fn sync_storage(
        &self,
    ) -> Result<(), closeclaw_session::persistence::PersistenceError> {
        self.session_manager.sync_storage().await
    }

    /// Close the storage backend and release resources (proxied to
    /// SessionManager).  Called during Phase 6 of daemon shutdown.
    pub async fn close_storage(
        &self,
    ) -> Result<(), closeclaw_session::persistence::PersistenceError> {
        self.session_manager.close_storage().await
    }

    /// Close outbound connections and clean up routing tables.
    ///
    /// Calls `shutdown_outbound()` on every registered IM plugin,
    /// clears the plugin registry, and drops the processor chain.
    /// Called during Phase 5 of daemon shutdown.
    pub async fn close_outbound(&self) {
        // Shutdown outbound for all registered plugins
        let plugins = self.get_all_plugins().await;
        for plugin in &plugins {
            if let Err(e) = plugin.shutdown_outbound().await {
                tracing::warn!(
                    platform = plugin.platform(),
                    error = %e,
                    "failed to shutdown plugin outbound — continuing"
                );
            }
        }
        // Clear plugin registry
        {
            let mut plugins = self.plugins.write().await;
            plugins.clear();
        }
        // Drop processor chain
        {
            let mut registry = self.processor_registry.write().unwrap();
            *registry = None;
        }
        tracing::info!("gateway outbound closed, routing table and processor registry cleared");
    }

    /// Register an IM plugin.
    ///
    /// The plugin's [`platform`](closeclaw_common::IMPlugin::platform) identifier is
    /// used as the registry key. Re-registering the same platform replaces
    /// the previous plugin.
    pub async fn register_plugin(&self, plugin: Arc<dyn closeclaw_common::IMPlugin>) {
        let key = plugin.platform().to_string();
        let mut plugins = self.plugins.write().await;
        plugins.insert(key, plugin);
    }

    /// Get a reference to the underlying SessionManager.
    pub fn session_manager(&self) -> &Arc<SessionManager> {
        &self.session_manager
    }

    /// Get a registered IM plugin by platform identifier.
    pub async fn get_plugin(&self, platform: &str) -> Option<Arc<dyn closeclaw_common::IMPlugin>> {
        let plugins = self.plugins.read().await;
        plugins.get(platform).cloned()
    }

    /// Get all registered IM plugins (snapshot).
    pub async fn get_all_plugins(&self) -> Vec<Arc<dyn closeclaw_common::IMPlugin>> {
        let plugins = self.plugins.read().await;
        plugins.values().cloned().collect()
    }

    /// Get active sessions for an agent (proxied to SessionManager).
    pub async fn get_agent_sessions(&self, agent_id: &str) -> Vec<Session> {
        self.session_manager.get_agent_sessions(agent_id).await
    }
}

/// Build a [`ProcessorRegistry`] with the standard inbound/outbound chains.
pub use processor_registry_builder::build_processor_registry;

/// Register the built-in outbound middlewares on a [`Gateway`].
use processor_registry_builder::register_default_middlewares;

#[cfg(test)]
mod anthropic_reasoning_chain_tests;
#[cfg(test)]
pub mod binding_resolution_tests;
#[cfg(test)]
pub mod compute_session_key_tests;
#[cfg(test)]
pub mod construction_tests;
#[cfg(test)]
pub mod gateway_alignment_tests;
#[cfg(test)]
pub mod inbound_chain_tests;
#[cfg(test)]
pub mod non_text_interception_tests;
#[cfg(test)]
pub mod notification_tests;
#[cfg(feature = "full-tests")]
#[path = "priority_prompt_tests.rs"]
pub mod priority_prompt_tests;
pub(crate) mod rebuild_stash;
#[cfg(test)]
mod rebuild_stash_tests;
#[cfg(test)]
pub mod session_handler_announce_reasoning_always_tests;
#[cfg(test)]
pub mod session_handler_circuit_breaker_tests;
#[cfg(test)]
pub mod session_handler_compact_config_tests;
#[cfg(test)]
pub mod session_handler_compact_truncate_tests;
#[cfg(feature = "full-tests")]
pub mod session_handler_dynamic_tests;
#[cfg(test)]
pub mod session_handler_recovery_tests;
#[cfg(test)]
pub mod session_handler_streaming_tests;
#[cfg(test)]
pub mod session_handler_tests;
#[cfg(test)]
pub mod session_routing_tests;
#[cfg(test)]
pub mod shutdown_handle_tests;
#[cfg(test)]
pub mod shutdown_phase_tests;
#[cfg(test)]
mod step1_3_tests;
#[cfg(test)]
pub mod step1_4_idle_verify_tests;
#[cfg(feature = "full-tests")]
pub mod step1_5_tests;
#[cfg(feature = "full-tests")]
pub mod tests;
#[cfg(feature = "full-tests")]
pub mod tests_dmscope;
#[cfg(feature = "full-tests")]
pub mod tests_processor_chain;
#[cfg(feature = "full-tests")]
pub mod tests_thread;
#[cfg(test)]
pub mod types_tests;
