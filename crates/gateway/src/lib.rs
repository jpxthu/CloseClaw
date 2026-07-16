//! Gateway - IM protocol adapters, message routing, authentication
//!
//! Central hub that connects IM platforms (Feishu, Discord, etc.) to agents.
pub mod approval;
#[cfg(test)]
pub mod approval_tests;
pub mod card_action;
pub(crate) mod health_check_builders;
#[cfg(test)]
mod health_check_builders_tests;
#[cfg(test)]
mod im_adapter;
pub mod inbound_queue;
#[cfg(test)]
mod inbound_queue_tests;
pub mod llm_caller_impl;
mod memory;
pub mod message;
pub mod outbound;
#[cfg(test)]
mod outbound_checkpoint_timing_tests;
#[cfg(test)]
mod outbound_fallback_tests;
#[cfg(test)]
mod outbound_tests;
#[cfg(test)]
mod receiving_transition_tests;
pub mod session_handler;
mod session_handler_announce;
mod session_handler_dispatch;
mod session_handler_streaming;
pub mod session_manager;
mod shutdown_card;
pub mod shutdown_handle;
pub mod slash_executor;
#[cfg(test)]
mod slash_executor_tests;
pub mod slash_permission;
#[cfg(test)]
mod slash_permission_tests;
#[cfg(test)]
mod streaming_pipeline_tests;
pub mod sweeper;
#[cfg(test)]
mod sweeper_tests;
#[cfg(test)]
pub mod tests_checkpoint;
#[cfg(feature = "full-tests")]
mod tests_plugin;
#[cfg(feature = "full-tests")]
mod tests_slash_permission;
pub mod types;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
pub use types::*;

use closeclaw_common::im_plugin::{MessageType, RenderedOutput};
use closeclaw_common::processor::ProcessedMessage;
pub use closeclaw_common::processor::ProcessorChain;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_common::slash_router::SlashRouter;
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
    checkpoint_manager: Option<Arc<CheckpointManager<dyn PersistenceService>>>,
    session_handler: Option<Arc<SessionMessageHandler>>,
    /// Daemon-level approval flow for intercepting `/approve` / `/deny` commands.
    approval_flow: RwLock<Option<Arc<tokio::sync::Mutex<ApprovalFlow>>>>,
    /// Slash command dispatcher.
    slash_dispatcher: RwLock<Option<Arc<dyn SlashRouter>>>,
    /// Permission engine for slash command authorization.
    permission_engine: RwLock<Option<Arc<tokio::sync::RwLock<PermissionEngine>>>>,
    /// Bounded inbound queue sender. `None` until the queue is started.
    inbound_tx: std::sync::Mutex<Option<mpsc::Sender<InboundRequest>>>,
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
}

impl Gateway {
    /// Create a new Gateway with the given config and a shared SessionManager.
    pub fn new(config: GatewayConfig, session_manager: Arc<SessionManager>) -> Self {
        Self {
            config,
            plugins: RwLock::new(HashMap::new()),
            session_manager,
            processor_registry: std::sync::RwLock::new(None),
            checkpoint_manager: None,
            session_handler: None,
            approval_flow: RwLock::new(None),
            slash_dispatcher: RwLock::new(None),
            permission_engine: RwLock::new(None),
            inbound_tx: std::sync::Mutex::new(None),
            self_ref: std::sync::Mutex::new(None),
            shutdown_handle: std::sync::Mutex::new(None),
            outbound_middlewares: std::sync::RwLock::new(Vec::new()),
            config_dir: RwLock::new(None),
            metrics_emitter: std::sync::RwLock::new(None),
        }
    }

    /// Create a new Gateway with the given config, SessionManager and ProcessorRegistry.
    pub fn with_processor_registry(
        config: GatewayConfig,
        session_manager: Arc<SessionManager>,
        registry: Arc<dyn ProcessorChain>,
    ) -> Self {
        Self {
            config,
            plugins: RwLock::new(HashMap::new()),
            session_manager,
            processor_registry: std::sync::RwLock::new(Some(registry)),
            checkpoint_manager: None,
            session_handler: None,
            approval_flow: RwLock::new(None),
            slash_dispatcher: RwLock::new(None),
            permission_engine: RwLock::new(None),
            inbound_tx: std::sync::Mutex::new(None),
            self_ref: std::sync::Mutex::new(None),
            shutdown_handle: std::sync::Mutex::new(None),
            outbound_middlewares: std::sync::RwLock::new(Vec::new()),
            config_dir: RwLock::new(None),
            metrics_emitter: std::sync::RwLock::new(None),
        }
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
        mut self,
        cm: Arc<CheckpointManager<dyn PersistenceService>>,
    ) -> Self {
        self.checkpoint_manager = Some(cm);
        self
    }

    /// Configure a SessionMessageHandler for busy/pending LLM session management.
    /// When a handler is installed, inbound messages are routed through the
    /// busy/pending state machine. When `None` (default), Gateway behaves as before.
    pub fn with_session_handler(mut self, handler: Arc<SessionMessageHandler>) -> Self {
        self.session_handler = Some(handler);
        self
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
        inbound_queue::start_inbound_consumer(rx, Arc::clone(self), capacity);
        inbound_queue::InboundQueueHandle::new(tx)
    }

    /// Enqueue an inbound request into the bounded queue.
    ///
    /// When the queue is full, a busy reply is sent via the IM plugin.
    /// If the queue has not been started, the message is processed
    /// directly (bypass mode).
    pub async fn enqueue_inbound(&self, request: inbound_queue::InboundRequest) {
        inbound_queue::enqueue_inbound(self, request).await;
    }

    /// Get a clone of the shutdown handle, if set.
    pub(crate) fn get_shutdown_handle(&self) -> Option<Arc<ShutdownHandle>> {
        self.shutdown_handle.lock().ok().and_then(|s| s.clone())
    }

    pub async fn has_slash_dispatcher(&self) -> bool {
        self.slash_dispatcher.read().await.is_some()
    }

    pub async fn has_session_handler(&self) -> bool {
        self.session_handler.is_some()
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

        // ── Resolve session_key → session_id ────────────────────────
        let session_id = match self.resolve_session_from_message(&processed, channel).await {
            Some(id) => id,
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
        if let Some(chat_id) = self
            .session_manager
            .take_restore_notification(&session_id)
            .await
        {
            if let Err(e) = self
                .send_outbound_simplified(&chat_id, channel, "正在恢复会话...")
                .await
            {
                tracing::warn!(
                    session_id = %session_id,
                    chat_id = %chat_id,
                    error = %e,
                    "failed to send restore notification"
                );
            }
        }

        let content = processed.text_content().unwrap_or("").to_string();

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
            .try_handle_approval_command(&session_id, &content, sender_id)
            .await
        {
            return Some(result);
        }

        // ── Slash command dispatch ─────────────────────────────────────
        // Slash commands are intercepted here and never appended to
        // conversation history (design doc requirement).
        if content.starts_with('/') {
            if let Some(result) = self
                .dispatch_slash(&session_id, &content, sender_id, channel)
                .await
            {
                return Some(result);
            }
        }

        let handler = self.session_handler.as_ref()?;

        // Streaming path: plugin is registered for this channel AND the
        // self-ref is wired AND the handler has a back-ref. Falls back
        // to the non-streaming path otherwise.
        let gw_arc = self
            .self_ref
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(Arc::clone));
        if let (Some(gw), Some(plugin)) = (gw_arc, self.get_plugin(channel).await) {
            let meta = crate::session_handler::MessageMetadata {
                sender_id: sender_id.unwrap_or("").to_string(),
                channel: channel.to_string(),
                timestamp: chrono::Utc::now().timestamp(),
            };
            let result = handler
                .handle_message_with_gateway(&session_id, content, meta, &gw, &plugin)
                .await;
            // NOTE: No decrement_busy here — the handler's spawned task
            // (finish_llm) is responsible for decrementing on async paths.
            if matches!(result, HandleResult::MessageQueued) && !peer_id.is_empty() {
                self.send_queuing_notification(&session_id, peer_id, channel)
                    .await;
            }
            return Some(result);
        }

        let result = handler.handle_message(&session_id, content).await;
        // NOTE: No decrement_busy here — the handler's spawned task
        // (finish_llm) is responsible for decrementing on async paths.
        if matches!(result, HandleResult::MessageQueued) && !peer_id.is_empty() {
            self.send_queuing_notification(&session_id, peer_id, channel)
                .await;
        }
        Some(result)
    }

    /// Send "⏳ 正在排队..." when a message is enqueued (session busy).
    async fn send_queuing_notification(&self, session_id: &str, peer_id: &str, channel: &str) {
        if let Err(e) = self
            .send_outbound_simplified(peer_id, channel, "⏳ 正在排队...")
            .await
        {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "failed to send queuing notification"
            );
        }
    }

    /// Resolve a session_id from a [`ProcessedMessage`]'s `session_key`.
    ///
    /// Extracts `session_key` from `metadata` and calls
    /// [`SessionManager::resolve`] to obtain the `session_id`.
    ///
    /// Returns `None` when:
    /// - `session_key` is missing or empty
    /// - [`SessionManager::resolve`] fails
    async fn resolve_session_from_message(
        &self,
        processed: &ProcessedMessage,
        channel: &str,
    ) -> Option<String> {
        let session_key = processed
            .metadata
            .get("session_key")
            .map(|s| s.as_str())
            .unwrap_or("");

        if session_key.is_empty() {
            tracing::warn!("session_key is empty — falling back to routing fields");
        }

        // Build a partial Message for SessionManager::resolve.
        // For existing sessions (key_registry hit), only thread_id is used.
        // For new sessions, to/from are needed for session creation.
        let peer_id = processed
            .metadata
            .get("peer_id")
            .map(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let sender_id = processed
            .metadata
            .get("sender_id")
            .map(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let message = Message {
            id: String::new(),
            from: sender_id,
            to: peer_id,
            content: processed.text_content().unwrap_or("").to_string(),
            channel: channel.to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            metadata: std::collections::HashMap::new(),
            thread_id: processed.metadata.get("thread_id").cloned(),
        };

        let account_id = processed.metadata.get("account_id").map(|s| s.as_str());

        self.session_manager
            .resolve(session_key, channel, &message, account_id)
            .await
            .ok()
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

    /// Route an incoming message to the appropriate agent.
    ///
    /// Supports two metadata formats for session resolution:
    /// 1. New path: `session_key` → call `SessionManager::resolve()` to get session_id
    /// 2. Old path: `session_id` → validate directly in active sessions table
    ///
    /// If both are missing, sends a user-visible error via the plugin and
    /// returns `NoRoutingKey`.
    /// Forward a resolved message to the IM plugin for the given channel.
    async fn forward_to_plugin(
        &self,
        channel: &str,
        message: &Message,
        session_id: &str,
    ) -> Result<(), GatewayError> {
        if !self.session_manager.has_session(session_id).await {
            return Err(GatewayError::MissingSessionId);
        }
        let plugin = self
            .get_plugin(channel)
            .await
            .ok_or(GatewayError::UnknownChannel(channel.to_string()))?;
        if message.content.len() > self.config.max_message_size {
            return Err(GatewayError::MessageTooLarge);
        }
        let thread_id = self.session_manager.get_thread_id(session_id).await;
        let output = RenderedOutput {
            msg_type: "text".into(),
            payload: json!({"content": {"text": &message.content}}),
        };
        plugin
            .send(&output, &message.to, thread_id.as_deref())
            .await
            .map_err(|e| GatewayError::AdapterError(e.to_string()))
    }

    /// Send a best-effort user-visible error via the plugin.
    async fn send_user_error(&self, channel: &str, message: &Message) {
        if let Some(plugin) = self.get_plugin(channel).await {
            let err_output = RenderedOutput {
                msg_type: "text".into(),
                payload: json!({
                    "content": {
                        "text":
                            "\u{4F1A}\u{8BDD}\u{8DEF}\u{7531}\u{5931}\u{8D25}\u{FF0C}\u{8BF7}\u{91CD}\u{8BD5}"
                    }
                }),
            };
            let _ = plugin.send(&err_output, &message.to, None).await;
        }
    }

    pub async fn route_message(
        &self,
        channel: &str,
        message: Message,
        account_id: Option<&str>,
    ) -> Result<(), GatewayError> {
        // --- New path: session_key → SessionManager::resolve() ---
        if let Some(session_key) = message.metadata.get("session_key") {
            if !session_key.is_empty() {
                let session_id = self
                    .session_manager
                    .resolve(session_key, channel, &message, account_id)
                    .await
                    .map_err(|e| GatewayError::AdapterError(e.to_string()))?;
                // Send restore notification through outbound chain (if any).
                if let Some(chat_id) = self
                    .session_manager
                    .take_restore_notification(&session_id)
                    .await
                {
                    if let Err(e) = self
                        .send_outbound_to_chat(&chat_id, channel, "正在恢复会话...")
                        .await
                    {
                        tracing::warn!(
                            session_id = %session_id,
                            error = %e,
                            "failed to send restore notification via outbound chain"
                        );
                    }
                }
                return self.forward_to_plugin(channel, &message, &session_id).await;
            }
        }

        // --- Fallback: session_id (old path, backward compatible) ---
        if let Some(session_id) = message.metadata.get("session_id") {
            if !session_id.is_empty() {
                return self.forward_to_plugin(channel, &message, session_id).await;
            }
        }

        // --- No key fallback: both missing/empty ---
        self.send_user_error(channel, &message).await;
        Err(GatewayError::NoRoutingKey)
    }

    /// Get active sessions for an agent (proxied to SessionManager).
    pub async fn get_agent_sessions(&self, agent_id: &str) -> Vec<Session> {
        self.session_manager.get_agent_sessions(agent_id).await
    }

    /// Runs the inbound processor chain on a [`NormalizedMessage`] built from `input`.
    /// Falls back to raw content on registry absence or processor error.
    pub async fn process_inbound_chain(&self, input: &InboundChainInput) -> ProcessedMessage {
        let extra_meta = build_extra_metadata(input);
        let registry = self.processor_registry.read().unwrap().clone();
        let Some(registry) = registry else {
            return ProcessedMessage {
                content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
                    input.content.to_string(),
                )],
                metadata: extra_meta,
            };
        };

        let normalized = closeclaw_common::im_plugin::NormalizedMessage {
            platform: input.platform.to_string(),
            sender_id: input.sender_id.to_string(),
            peer_id: input.peer_id.to_string(),
            content: input.content.to_string(),
            timestamp: input.timestamp_ms,
            message_type: input.message_type.clone(),
            media_refs: input.media_refs.clone(),
            thread_id: input.thread_id.clone(),
            account_id: input.account_id.clone().unwrap_or_default(),
        };

        match registry.process_inbound(normalized).await {
            Ok(mut processed) => {
                processed.metadata.extend(extra_meta);
                processed
            }
            Err(e) => {
                tracing::warn!(?e, "processor chain failed, falling back to raw content");
                ProcessedMessage {
                    content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
                        input.content.to_string(),
                    )],
                    metadata: extra_meta,
                }
            }
        }
    }
}

/// Build extra metadata map from inbound chain input fields.
///
/// Propagates `thread_id`, `message_type`, and `media_refs`
/// so they are available downstream in the Gateway.
fn build_extra_metadata(input: &InboundChainInput) -> std::collections::HashMap<String, String> {
    let mut meta = std::collections::HashMap::new();
    if let Some(ref thread_id) = input.thread_id {
        meta.insert("thread_id".to_string(), thread_id.clone());
    }
    meta.insert(
        "message_type".to_string(),
        serde_json::to_string(&input.message_type).unwrap_or_else(|_| "text".to_string()),
    );
    meta.insert(
        "media_refs".to_string(),
        serde_json::to_string(&input.media_refs).unwrap_or_else(|_| "[]".to_string()),
    );
    if let Some(ref account_id) = input.account_id {
        meta.insert("account_id".to_string(), account_id.clone());
    }
    meta
}
#[cfg(test)]
pub mod compute_session_key_tests;
#[cfg(test)]
pub mod inbound_chain_tests;
#[cfg(test)]
pub mod non_text_interception_tests;
#[cfg(test)]
pub mod notification_tests;
#[cfg(feature = "full-tests")]
#[path = "priority_prompt_tests.rs"]
pub mod priority_prompt_tests;
#[cfg(feature = "full-tests")]
pub mod session_handler_dynamic_tests;
#[cfg(test)]
pub mod session_handler_recovery_tests;
#[cfg(test)]
pub mod session_handler_tests;
#[cfg(test)]
pub mod session_routing_tests;
#[cfg(test)]
pub mod shutdown_handle_tests;
#[cfg(test)]
pub mod shutdown_phase_tests;
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
