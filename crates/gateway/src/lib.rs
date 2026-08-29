//! Gateway - IM protocol adapters, message routing, authentication
//!
//! Central hub that connects IM platforms (Feishu, Discord, etc.) to agents.
pub mod approval;
#[cfg(test)]
pub mod approval_tests;
pub mod card_action;
mod debug_log_emitter;
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

#[cfg(test)]
mod tests_slash_dispatcher_routing;
#[cfg(feature = "full-tests")]
mod tests_slash_permission;
#[cfg(feature = "full-tests")]
mod tests_slash_permission_integration;
pub mod types;
pub mod workflow_owner;
pub use outbound::OutboundMeta;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
pub use types::*;

use closeclaw_common::im_plugin::MessageType;
use closeclaw_common::processor::ProcessedMessage;
pub use closeclaw_common::processor::ProcessorChain;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_common::slash_router::SlashRouter;
use closeclaw_debug_log::{DebugLog, LogLevel};
use closeclaw_llm::ProviderModelKnowledge;
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_session::checkpoint_manager::CheckpointManager;
use closeclaw_session::persistence::PersistenceService;
pub use inbound_queue::{InboundQueueFull, InboundQueueHandle, InboundRequest};
pub use session_handler::{HandleResult, SessionMessageHandler};
pub use session_manager::{SessionManager, SpawnController};
pub use shutdown_handle::ShutdownHandle;

/// Gateway - routes messages between IM plugins and agents
pub struct Gateway {
    config: GatewayConfig,
    plugins: RwLock<HashMap<String, Arc<dyn closeclaw_common::IMPlugin>>>,
    session_manager: Arc<SessionManager>,
    processor_registry: std::sync::RwLock<Option<Arc<dyn ProcessorChain>>>,
    checkpoint_manager: std::sync::RwLock<Option<Arc<CheckpointManager<dyn PersistenceService>>>>,
    session_handler: std::sync::OnceLock<Arc<SessionMessageHandler>>,
    /// Daemon-level approval flow for intercepting `/approve` / `/deny` commands.
    approval_flow: RwLock<Option<Arc<tokio::sync::Mutex<ApprovalFlow>>>>,
    /// Slash command dispatcher.
    slash_dispatcher: RwLock<Option<Arc<dyn SlashRouter>>>,
    /// Permission engine for slash command authorization.
    permission_engine: RwLock<Option<Arc<tokio::sync::RwLock<PermissionEngine>>>>,
    /// Bounded inbound queue sender. `None` until the queue is started.
    inbound_tx: std::sync::Mutex<Option<mpsc::Sender<inbound_queue::QueuedInbound>>>,
    /// Self-reference for back-pointer to the owning `Arc<Gateway>`.
    /// `handle_inbound_message` is called with `&self`, but
    /// `SessionMessageHandler` needs an `Arc<Gateway>` to call
    /// `send_outbound_streaming`. The caller wires this after wrapping
    /// the `Gateway` in `Arc::new(...)` via `set_self_ref`. Until set,
    /// the slot is `None`; the handler falls back to the non-streaming
    /// path in that case.
    self_ref: std::sync::Mutex<Option<Arc<Gateway>>>,
    /// Shutdown handle for busy-count tracking during drain.
    shutdown_handle: std::sync::Mutex<Option<Arc<ShutdownHandle>>>,
    /// Outbound middleware chain, run between render and send.
    outbound_middlewares: std::sync::RwLock<Vec<Arc<dyn closeclaw_common::OutboundMiddleware>>>,
    /// Config directory for permission rule persistence.
    config_dir: RwLock<Option<std::path::PathBuf>>,
    /// Metrics emitter for operational metrics (cache breaks, etc.).
    metrics_emitter: std::sync::RwLock<Option<Arc<dyn closeclaw_common::MetricsEmitter>>>,
    /// Debug log framework instance for structured event logging.
    debug_log: std::sync::RwLock<Option<DebugLog>>,
    /// WAL persistence for inbound queue durability.
    /// `None` when `inbound_wal_dir` is not configured.
    inbound_wal: std::sync::Mutex<Option<Arc<inbound_wal::InboundWal>>>,
}

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
        };
        register_default_middlewares(&gw, &gw.config);
        gw
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

    /// Wire the back-reference to the owning `Arc<Gateway>`.
    ///
    /// Call this immediately after `Arc::new(Gateway::new(...))` so that
    /// `handle_inbound_message` can pass a strong `Arc<Gateway>` to the
    /// session handler for streaming dispatch.
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

    // set_slash_dispatcher, set_permission_engine, and set_approval_flow
    // are defined in slash_permission.rs and approval.rs respectively.

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
        debug_log_emitter::emit_debug_event(
            guard.as_ref(),
            trace_id,
            None,
            closeclaw_debug_log::LogLevel::Info,
            "gateway",
            "queue.replayed",
            serde_json::json!({
                "platform": platform,
                "peer_id": peer_id,
            }),
        );
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

    /// Handle an inbound message through the busy/pending state machine.
    ///
    /// Resolution flow: extract `session_key` → resolve `session_id` →
    /// dispatch slash commands or route to LLM. Slash commands are intercepted
    /// here and never appended to conversation history.
    ///
    /// When a plugin is registered for `channel` AND the self-ref is wired,
    /// dispatches through `handle_message_with_gateway` for streaming;
    /// otherwise falls back to non-streaming `handle_message`.
    pub async fn handle_inbound_message(
        &self,
        processed: ProcessedMessage,
        sender_id: Option<&str>,
        channel: &str,
    ) -> Option<HandleResult> {
        // ── Extract peer_id once for reuse ──────────────────────────
        let peer_id = processed
            .metadata
            .get("peer_id")
            .map(|s| s.as_str())
            .unwrap_or("");

        // ── Debug log: message.arrived ──────────────────────────────
        if let Some(trace_id) = processed.metadata.get("trace_id") {
            let guard = self.debug_log.read().unwrap_or_else(|e| e.into_inner());
            debug_log_emitter::emit_debug_event(
                guard.as_ref(),
                trace_id,
                processed.metadata.get("session_key").map(|s| s.as_str()),
                LogLevel::Info,
                "gateway",
                "message.arrived",
                serde_json::json!({
                    "sender_id": sender_id.unwrap_or(""),
                    "peer_id": peer_id,
                    "channel": channel,
                }),
            );
        }

        // ── Non-text message interception (before session resolution) ─
        // Per design doc: non-text messages (image/file/audio) get a
        // simplified outbound reply and must NOT trigger session resolution.
        let message_type: MessageType = processed
            .metadata
            .get("message_type")
            .and_then(|s| serde_json::from_str::<MessageType>(s).ok())
            .unwrap_or_default();
        if !matches!(message_type, MessageType::Text) {
            tracing::info!(
                message_type = ?message_type,
                "rejecting non-text message"
            );
            if let Err(e) = self
                .send_outbound_simplified(
                    peer_id,
                    channel,
                    "\u{6682}\u{4E0D}\u{652F}\u{6301}\u{8BE5}\u{6D88}\u{606F}\u{7C7B}\u{578B}",
                )
                .await
            {
                tracing::warn!(
                    error = %e,
                    "failed to send non-text rejection reply"
                );
            }
            return None;
        }

        // ── Extract content early for size check and downstream use ─
        let content = processed.text_content().unwrap_or("").to_string();

        // ── max_message_size enforcement ────────────────────────────
        // Per design doc: GatewayConfig includes max_message_size;
        // messages exceeding the limit are rejected with a simplified
        // reply before session resolution to protect downstream resources.
        if content.len() > self.config.max_message_size {
            tracing::warn!(
                peer_id = %peer_id,
                size = content.len(),
                limit = self.config.max_message_size,
                "inbound message exceeds max_message_size"
            );
            if !peer_id.is_empty() {
                if let Err(e) = self
                    .send_outbound_simplified(peer_id, channel, "消息过长，请缩短后重试")
                    .await
                {
                    tracing::warn!(
                        error = %e,
                        "failed to send max_message_size rejection reply"
                    );
                }
            }
            return None;
        }

        // ── Resolve session_key → session_id ────────────────────────
        let trace_id = processed.metadata.get("trace_id").map(|s| s.as_str());
        let session_id = match self.resolve_session_from_message(&processed, channel).await {
            Some(id) => {
                // ── Debug log: session.resolved ─────────────────────────
                if let Some(tid) = trace_id {
                    let guard = self.debug_log.read().unwrap_or_else(|e| e.into_inner());
                    debug_log_emitter::emit_debug_event(
                        guard.as_ref(),
                        tid,
                        processed.metadata.get("session_key").map(|s| s.as_str()),
                        LogLevel::Info,
                        "gateway",
                        "session.resolved",
                        serde_json::json!({
                            "session_id": id,
                            "channel": channel,
                        }),
                    );
                }
                id
            }
            None => {
                tracing::warn!("session_key missing or resolve failed — message not processed");
                if !peer_id.is_empty() {
                    if let Err(e) = self
                        .send_outbound_simplified(peer_id, channel, "\u{4F1A}\u{8BDD}\u{8DEF}\u{7531}\u{5931}\u{8D25}\u{FF0C}\u{8BF7}\u{91CD}\u{8BD5}")
                        .await
                    {
                        tracing::warn!(
                            error = %e,
                            "failed to send session routing failure reply"
                        );
                    }
                }
                return None;
            }
        };

        // ── Restore notification for archived sessions ──────────────
        // Per design doc: when a session is restored from archived state,
        // send "正在恢复会话..." before processing continues.
        // When migrating, send the custom archiving notification instead.
        if let Some((chat_id, custom_msg)) = self
            .session_manager
            .take_restore_notification(&session_id)
            .await
        {
            let msg = custom_msg.as_deref().unwrap_or("正在恢复会话...");
            if let Err(e) = self.send_outbound_simplified(&chat_id, channel, msg).await {
                tracing::warn!(
                    session_id = %session_id,
                    chat_id = %chat_id,
                    error = %e,
                    "failed to send restore notification"
                );
            }
        }

        // ── Shutdown gate: reject new operations ──────────────────────
        if let Some(sh) = self.get_shutdown_handle() {
            if sh.is_shutting_down() {
                tracing::warn!(
                    session_id = %session_id,
                    "rejecting inbound message: daemon is shutting down"
                );
                return None;
            }
        }

        // ── Session stopped gate: reject new messages ─────────────────
        // Per design doc: during graceful stop the `stopped` flag is set
        // to prevent new LLM requests. New user messages are rejected
        // (dropped) so they don't trigger autonomous turns.
        if let Some(cs) = self
            .session_manager
            .get_conversation_session(&session_id)
            .await
        {
            if cs.read().await.is_stopped() {
                tracing::warn!(
                    session_id = %session_id,
                    "rejecting inbound message: session is stopped"
                );
                return None;
            }
        }

        // ── New user auto-registration ─────────────────────────────────
        // Per design doc: when a non-owner, unregistered user sends
        // their first message, auto-submit a user creation request for
        // Owner approval. The user is blocked until approved.
        if let Some(sender) = sender_id {
            if let Some(result) = self.check_new_user_registration(sender, channel).await {
                return Some(result);
            }
        }

        // ── Approval command interception ──────────────────────────────
        if let Some(result) = self
            .try_handle_approval_command(&session_id, &content, sender_id, peer_id, channel)
            .await
        {
            return Some(result);
        }

        // ── Routing decision ────────────────────────────────────────────
        // Log route.decision before dispatching slash or normal message path.
        let is_slash = content.starts_with('/');
        if let Some(tid) = trace_id {
            let guard = self.debug_log.read().unwrap_or_else(|e| e.into_inner());
            debug_log_emitter::emit_debug_event(
                guard.as_ref(),
                tid,
                processed.metadata.get("session_key").map(|s| s.as_str()),
                LogLevel::Info,
                "gateway",
                "route.decision",
                serde_json::json!({
                    "session_id": session_id,
                    "decision": if is_slash { "slash" } else { "normal" },
                    "content_prefix": content.chars().take(16).collect::<String>(),
                }),
            );
        }
        if is_slash {
            // ── Slash command dispatch ─────────────────────────────────────
            // Slash commands are intercepted here and never appended to
            // conversation history (design doc requirement).
            if let Some(result) = self
                .dispatch_slash(&session_id, &content, sender_id, channel, Some(peer_id))
                .await
            {
                return Some(result);
            }
        }

        // ── Owner response for blocked workflow (Step 1.6) ──────────
        // When a workflow is in blocked state, owner messages are
        // intercepted and routed to the engine for resolve or terminate.
        if let Some(result) = self
            .try_handle_workflow_owner_response(&session_id, &content, sender_id)
            .await
        {
            return Some(result);
        }

        let handler = self.session_handler.get().cloned()?;

        // Streaming path: plugin is registered for this channel AND the
        // self-ref is wired AND the handler has a back-ref. Falls back
        // to the non-streaming path otherwise.
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
                trace_id: trace_id.map(|s| s.to_string()),
                session_key: processed.metadata.get("session_key").cloned(),
            };
            let result = handler
                .handle_message_with_gateway(&session_id, content, meta, &gw, &plugin)
                .await;
            // NOTE: No decrement_busy here — the handler's spawned task
            // (finish_llm) is responsible for decrementing on async paths.
            self.maybe_send_notification(&result, peer_id, channel)
                .await;
            return Some(result);
        }

        let result = handler.handle_message(&session_id, content).await;
        // NOTE: No decrement_busy here — the handler's spawned task
        // (finish_llm) is responsible for decrementing on async paths.
        self.maybe_send_notification(&result, peer_id, channel)
            .await;
        Some(result)
    }

    /// Send a notification to the user when the result carries a message
    /// (e.g. queuing, error).
    async fn send_notification(&self, peer_id: &str, channel: &str, text: &str) {
        if let Err(e) = self.send_outbound_simplified(peer_id, channel, text).await {
            tracing::warn!(
                peer_id = %peer_id,
                error = %e,
                "failed to send notification"
            );
        }
    }

    /// If `result` carries a user-facing message (`MessageQueued` or `Error`),
    /// send it as a notification.  No-op for other variants or empty peer_id.
    async fn maybe_send_notification(&self, result: &HandleResult, peer_id: &str, channel: &str) {
        if peer_id.is_empty() {
            return;
        }
        let text = match result {
            HandleResult::MessageQueued(t) => t,
            HandleResult::Error(t) => t,
            _ => return,
        };
        self.send_notification(peer_id, channel, text).await;
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
