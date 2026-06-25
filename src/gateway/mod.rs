//! Gateway - IM protocol adapters, message routing, authentication
//!
//! Central hub that connects IM platforms (Feishu, Discord, etc.) to agents.

pub mod approval;
pub mod inbound_queue;
pub mod message;
pub mod outbound;
pub mod session_handler;
mod session_handler_announce;
mod session_handler_dispatch;
mod session_handler_streaming;
pub mod session_manager;
pub mod slash_permission;

#[cfg(test)]
mod inbound_queue_tests;
#[cfg(test)]
mod outbound_tests;
#[cfg(test)]
mod tests_plugin;
#[cfg(test)]
mod tests_slash_permission;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use crate::permission::approval_flow::ApprovalFlow;
use crate::permission::engine::engine_eval::PermissionEngine;
use crate::renderer::RenderedOutput;
use crate::session::checkpoint_manager::CheckpointManager;
use crate::session::persistence::PersistenceService;
use crate::slash::SlashDispatcher;

use crate::processor_chain::context::{ProcessedMessage, RawMessage};
pub use crate::processor_chain::ProcessorRegistry;
pub use inbound_queue::{InboundQueueFull, InboundQueueHandle, InboundRequest};
pub use session_handler::{HandleResult, SessionMessageHandler};
pub use session_manager::SessionManager;

use sha2::{Digest, Sha256};

use crate::llm::types::ContentBlock;

/// Type alias for the output channel sender used across session handler modules.
pub(crate) type OutputTx = Arc<RwLock<Option<mpsc::Sender<(String, Vec<ContentBlock>)>>>>;

/// DM session scope - controls how session keys are partitioned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DmScope {
    /// Single shared session for all peers on a channel (backward compatible)
    Main,
    /// One session per peer pair (from → to)
    PerPeer,
    /// One session per channel + peer pair
    PerChannelPeer,
    /// One session per account + channel + peer pair
    PerAccountChannelPeer,
    /// One session per channel + sender (excludes `to` field).
    ///
    /// Used when agent-level isolation is provided by a per-agent
    /// [`SessionManager`] — the session key is `{channel}:{from}`, so
    /// different agents sharing the same channel naturally stay isolated
    /// without embedding `agent_id` in the key.
    PerChannelSender,
}

#[allow(clippy::derivable_impls)]
impl Default for DmScope {
    fn default() -> Self {
        DmScope::PerAccountChannelPeer
    }
}

impl DmScope {
    /// Compute a session key for the given context.
    ///
    /// Format: `{timestamp_ms}-{sha256_hex(routing_fields)}`
    /// where `routing_fields` varies by scope variant.
    pub fn compute_session_key(
        &self,
        channel: &str,
        message: &Message,
        account_id: Option<&str>,
        timestamp_ms: i64,
    ) -> String {
        let routing_fields = match self {
            DmScope::Main => {
                format!("{}:{}", channel, message.to)
            }
            DmScope::PerPeer => {
                format!("{}:{}", message.from, message.to)
            }
            DmScope::PerChannelPeer => {
                format!("{}:{}:{}", channel, message.from, message.to)
            }
            DmScope::PerAccountChannelPeer => {
                let acc = account_id.unwrap_or("default");
                format!("{}:{}:{}:{}", channel, message.from, message.to, acc)
            }
            DmScope::PerChannelSender => {
                format!("{}:{}", channel, message.from)
            }
        };
        let hash = Sha256::digest(routing_fields.as_bytes());
        format!("{}-{:x}", timestamp_ms, hash)
    }
}

/// Internal message representation - all IM messages are converted to this
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Message {
    pub id: String,
    pub from: String,
    pub to: String,
    pub content: String,
    pub channel: String,
    pub timestamp: i64,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    #[serde(default)]
    pub thread_id: Option<String>,
}

/// Gateway configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayConfig {
    pub name: String,
    #[serde(default)]
    pub rate_limit_per_minute: u32,
    #[serde(default)]
    pub max_message_size: usize,
    #[serde(default)]
    pub dm_scope: DmScope,
    /// Directory for raw inbound log files.
    /// When `None` (default), raw logging is disabled.
    #[serde(default)]
    pub raw_log_dir: Option<std::path::PathBuf>,
    /// Maximum number of messages the inbound queue can buffer.
    /// Defaults to 64.
    #[serde(default = "default_inbound_queue_capacity")]
    pub inbound_queue_capacity: usize,
}

fn default_inbound_queue_capacity() -> usize {
    64
}

#[allow(clippy::derivable_impls)]
impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            rate_limit_per_minute: 0,
            max_message_size: 0,
            dm_scope: DmScope::default(),
            raw_log_dir: None,
            inbound_queue_capacity: default_inbound_queue_capacity(),
        }
    }
}

/// Session - represents an active conversation
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub agent_id: String,
    pub channel: String,
    pub created_at: i64,
    /// Nesting depth. 0 for root sessions, parent.depth + 1 for child sessions.
    pub depth: u32,
}

/// Gateway - routes messages between IM plugins and agents
pub struct Gateway {
    config: GatewayConfig,
    plugins: RwLock<HashMap<String, Arc<dyn super::im::IMPlugin>>>,
    session_manager: Arc<SessionManager>,
    processor_registry: Option<Arc<ProcessorRegistry>>,
    checkpoint_manager: Option<Arc<CheckpointManager<dyn PersistenceService>>>,
    session_handler: Option<Arc<SessionMessageHandler>>,
    /// Daemon-level approval flow for intercepting `/approve` / `/deny` commands.
    approval_flow: RwLock<Option<Arc<tokio::sync::Mutex<ApprovalFlow>>>>,
    /// Slash command dispatcher.
    slash_dispatcher: RwLock<Option<Arc<SlashDispatcher>>>,
    /// Permission engine for slash command authorization.
    permission_engine: RwLock<Option<Arc<PermissionEngine>>>,
    /// Bounded inbound queue sender. `None` until the queue is started.
    #[allow(dead_code)]
    inbound_tx: std::sync::Mutex<Option<mpsc::Sender<InboundRequest>>>,
    /// Self-reference for back-pointer to the owning `Arc<Gateway>`.
    ///
    /// `handle_inbound_message` is called with `&self`, but
    /// `SessionMessageHandler` needs an `Arc<Gateway>` to call
    /// `send_outbound_streaming`. The caller wires this after wrapping
    /// the `Gateway` in `Arc::new(...)` via `set_self_ref`. Until set,
    /// the slot is `None`; the handler falls back to the non-streaming
    /// path in that case.
    self_ref: std::sync::Mutex<Option<Arc<Gateway>>>,
    /// Shutdown handle for busy-count tracking during drain.
    shutdown_handle: std::sync::Mutex<Option<Arc<crate::daemon::shutdown::ShutdownHandle>>>,
}

impl Gateway {
    /// Create a new Gateway with the given config and a shared SessionManager.
    pub fn new(config: GatewayConfig, session_manager: Arc<SessionManager>) -> Self {
        Self {
            config,
            plugins: RwLock::new(HashMap::new()),
            session_manager,
            processor_registry: None,
            checkpoint_manager: None,
            session_handler: None,
            approval_flow: RwLock::new(None),
            slash_dispatcher: RwLock::new(None),
            permission_engine: RwLock::new(None),
            inbound_tx: std::sync::Mutex::new(None),
            self_ref: std::sync::Mutex::new(None),
            shutdown_handle: std::sync::Mutex::new(None),
        }
    }

    /// Create a new Gateway with the given config, SessionManager and ProcessorRegistry.
    pub fn with_processor_registry(
        config: GatewayConfig,
        session_manager: Arc<SessionManager>,
        registry: Arc<ProcessorRegistry>,
    ) -> Self {
        Self {
            config,
            plugins: RwLock::new(HashMap::new()),
            session_manager,
            processor_registry: Some(registry),
            checkpoint_manager: None,
            session_handler: None,
            approval_flow: RwLock::new(None),
            slash_dispatcher: RwLock::new(None),
            permission_engine: RwLock::new(None),
            inbound_tx: std::sync::Mutex::new(None),
            self_ref: std::sync::Mutex::new(None),
            shutdown_handle: std::sync::Mutex::new(None),
        }
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
    ///
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
    pub fn set_shutdown_handle(&self, handle: Arc<crate::daemon::shutdown::ShutdownHandle>) {
        if let Ok(mut slot) = self.shutdown_handle.lock() {
            *slot = Some(handle);
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
    pub(crate) fn get_shutdown_handle(
        &self,
    ) -> Option<Arc<crate::daemon::shutdown::ShutdownHandle>> {
        self.shutdown_handle.lock().ok().and_then(|s| s.clone())
    }

    #[cfg(test)]
    pub(crate) fn processor_registry_len(&self) -> (usize, usize) {
        self.processor_registry
            .as_ref()
            .map(|r| (r.inbound_len(), r.outbound_len()))
            .unwrap_or((0, 0))
    }

    #[cfg(test)]
    pub(crate) async fn has_slash_dispatcher(&self) -> bool {
        self.slash_dispatcher.read().await.is_some()
    }

    #[cfg(test)]
    pub(crate) async fn has_session_handler(&self) -> bool {
        self.session_handler.is_some()
    }

    #[cfg(test)]
    pub(crate) fn config_name(&self) -> &str {
        &self.config.name
    }

    /// Handle an inbound message through the busy/pending state machine.
    ///
    /// Accepts a [`ProcessedMessage`] produced by the inbound processor
    /// chain (containing cleaned `content` and `session_key` in metadata).
    ///
    /// Resolution flow:
    /// 1. Extract `session_key` from `metadata`
    /// 2. If empty → reply "会话路由失败" via plugin, return `None`
    /// 3. Call [`SessionManager::resolve`] to obtain `session_id`
    /// 4. Dispatch: slash commands → [`dispatch_slash`], normal → LLM
    ///
    /// Slash commands are intercepted at this layer and never appended
    /// to the conversation history (design doc requirement).
    ///
    /// `channel` identifies the IM platform / channel the message originated
    /// from (e.g. `"feishu"`). It is forwarded to `dispatch_slash` so that
    /// `SlashContext.channel` reflects the real source.
    ///
    /// When a plugin is registered for `channel` AND the self-ref is wired
    /// (see [`set_self_ref`](Self::set_self_ref)), this dispatches through
    /// [`SessionMessageHandler::handle_message_with_gateway`] so streaming
    /// LLM output can flow through [`Gateway::send_outbound_streaming`].
    /// Otherwise it falls back to the non-streaming path
    /// [`SessionMessageHandler::handle_message`].
    ///
    /// Returns `HandleResult` (`LlmStarted`/`MessageQueued`/`ApprovalProcessed`),
    /// or `None` if no handler configured.
    pub async fn handle_inbound_message(
        &self,
        processed: ProcessedMessage,
        sender_id: Option<&str>,
        channel: &str,
    ) -> Option<HandleResult> {
        // ── Resolve session_key → session_id ────────────────────────
        let session_id = match self.resolve_session_from_message(&processed, channel).await {
            Some(id) => id,
            None => {
                tracing::warn!("session_key missing or resolve failed — message not processed");
                // Reply to user with error per design doc
                let peer_id = processed
                    .metadata
                    .get("peer_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !peer_id.is_empty() {
                    let err_msg = Message {
                        id: String::new(),
                        from: String::new(),
                        to: peer_id.to_string(),
                        content: String::new(),
                        channel: channel.to_string(),
                        timestamp: 0,
                        metadata: std::collections::HashMap::new(),
                        thread_id: None,
                    };
                    self.send_user_error(channel, &err_msg).await;
                }
                return None;
            }
        };

        let content = processed.content;

        // ── Card action interception ─────────────────────────────────
        // Must run before the approval command check so that Feishu card
        // action callbacks (e.g. "Forceful shutdown" button) are handled
        // even when the daemon is already shutting down.
        if content.starts_with("/__card_action:forceful_shutdown") {
            if let Some(sh) = self.get_shutdown_handle() {
                if sh.is_shutting_down() {
                    tracing::info!(
                        session_id = %session_id,
                        "card action: escalating to forceful shutdown"
                    );
                    sh.escalate_to_forceful();
                }
            }
            return None;
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
            let meta = crate::gateway::session_handler::MessageMetadata {
                sender_id: sender_id.unwrap_or("").to_string(),
                channel: channel.to_string(),
                timestamp: chrono::Utc::now().timestamp(),
            };
            let result = handler
                .handle_message_with_gateway(&session_id, content, meta, &gw, &plugin)
                .await;
            // NOTE: No decrement_busy here — the handler's spawned task
            // (finish_llm) is responsible for decrementing on async paths.
            return Some(result);
        }

        let result = handler.handle_message(&session_id, content).await;
        // NOTE: No decrement_busy here — the handler's spawned task
        // (finish_llm) is responsible for decrementing on async paths.
        Some(result)
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
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if session_key.is_empty() {
            tracing::warn!("session_key is empty — session routing failed");
            return None;
        }

        // Build a partial Message for SessionManager::resolve.
        // For existing sessions (key_registry hit), only thread_id is used.
        // For new sessions, to/from are needed for session creation.
        let peer_id = processed
            .metadata
            .get("peer_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let sender_id = processed
            .metadata
            .get("sender_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let message = Message {
            id: String::new(),
            from: sender_id,
            to: peer_id,
            content: processed.content.clone(),
            channel: channel.to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            metadata: std::collections::HashMap::new(),
            thread_id: None,
        };

        self.session_manager
            .resolve(session_key, channel, &message, None)
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
        mode: crate::daemon::shutdown::ShutdownMode,
    ) -> Result<usize, crate::session::persistence::PersistenceError> {
        self.session_manager.flush_all(mode).await
    }

    /// Register an IM plugin.
    ///
    /// The plugin's [`platform`](super::im::IMPlugin::platform) identifier is
    /// used as the registry key. Re-registering the same platform replaces
    /// the previous plugin.
    pub async fn register_plugin(&self, plugin: Arc<dyn super::im::IMPlugin>) {
        let key = plugin.platform().to_string();
        let mut plugins = self.plugins.write().await;
        plugins.insert(key, plugin);
    }

    /// Get a reference to the underlying SessionManager.
    pub fn session_manager(&self) -> &Arc<SessionManager> {
        &self.session_manager
    }

    /// Get a registered IM plugin by platform identifier.
    pub async fn get_plugin(&self, platform: &str) -> Option<Arc<dyn super::im::IMPlugin>> {
        let plugins = self.plugins.read().await;
        plugins.get(platform).cloned()
    }

    /// Get all registered IM plugins (snapshot).
    pub async fn get_all_plugins(&self) -> Vec<Arc<dyn super::im::IMPlugin>> {
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
                            "\u{26A0}\u{FE0F} \u{4F1A}\u{8BDD}\u{8DEF}\u{7531}\
                            \u{5931}\u{8D25}\u{FF0C}\u{8BF7}\u{91CD}\u{8BD5}"
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

    /// Build and send a shutdown progress card to all active session chats.
    ///
    /// Displays per-session status (LLM streaming / tool executing / idle)
    /// and elapsed wait time. The card includes [Continue waiting] and
    /// [Force close] buttons. Sending failures are logged as warnings and
    /// do not block the shutdown flow.
    ///
    /// When `mode` is [`ShutdownMode::Forceful`], the header changes to
    /// indicate forced shutdown and the action buttons are omitted.
    pub async fn send_shutdown_progress_card(&self, mode: crate::daemon::shutdown::ShutdownMode) {
        use crate::llm::session_state::LlmState;

        let sessions = self.session_manager.get_all_sessions().await;
        if sessions.is_empty() {
            return;
        }

        // First pass: group sessions by chat_id, drop read lock before second pass.
        let mut chats: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for session in &sessions {
            if let Some(chat_id) = self.session_manager.get_chat_id(&session.id).await {
                chats.entry(chat_id).or_default().push(session.id.clone());
            }
        }

        let mut session_elements: Vec<serde_json::Value> = Vec::new();
        for session in &sessions {
            // Re-acquire conv_sessions read lock per session to avoid
            // holding it across the entire loop (fixes E2 review item 1).
            let conv_sessions = self.session_manager.conversation_sessions.read().await;
            let (status_text, activity_at) = match conv_sessions.get(&session.id) {
                Some(cs) => {
                    let guard = cs.read().await;
                    let state = *guard.llm_state.read().expect("llm_state lock poisoned");
                    let activity = guard.last_activity_at();
                    let has_running_tool = {
                        let tool_states =
                            guard.tool_states.read().expect("tool_states lock poisoned");
                        tool_states.values().any(|s| {
                            matches!(
                                s,
                                crate::llm::session_state::ToolExecState::RunningForeground
                                    | crate::llm::session_state::ToolExecState::RunningBackground
                            )
                        })
                    };
                    drop(guard);
                    let label = if has_running_tool {
                        "\u{1f527} \u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}"
                    } else if matches!(state, LlmState::Requesting | LlmState::Receiving) {
                        "\u{1f4ac} LLM \u{6d41}\u{5f0f}\u{8f93}\u{51fa}\u{4e2d}"
                    } else {
                        "\u{2705} Idle"
                    };
                    (label, activity)
                }
                None => ("\u{2705} Idle", session.created_at),
            };
            drop(conv_sessions);

            let elapsed = {
                let now = chrono::Utc::now().timestamp();
                let secs = (now - activity_at).max(0) as u64;
                if secs < 60 {
                    format!("{}s", secs)
                } else {
                    format!("{}m{}s", secs / 60, secs % 60)
                }
            };

            session_elements.push(json!({
                "tag": "div",
                "text": json!({
                    "tag": "lark_md",
                    "content": format!(
                        "\u{2022} `{}` \u{2014} {} (\u{5df2}\u{7b49}\u{5f85} {})",
                        session.id, status_text, elapsed
                    )
                })
            }));
        }

        // Action buttons (only in graceful mode)
        let mut elements = session_elements;
        if mode == crate::daemon::shutdown::ShutdownMode::Graceful {
            elements.push(json!({
                "tag": "action",
                "actions": [
                    json!({
                        "tag": "button",
                        "text": json!({
                            "tag": "plain_text",
                            "content": "\u{7ee7}\u{7eed}\u{7b49}\u{5f85}"
                        }),
                        "type": "default",
                        "disabled": true
                    }),
                    json!({
                        "tag": "button",
                        "text": json!({
                            "tag": "plain_text",
                            "content": "\u{5f3a}\u{5236}\u{5173}\u{95ed}"
                        }),
                        "type": "danger",
                        "value": {"action": "forceful_shutdown"}
                    })
                ]
            }));
        }

        let header_title = if mode == crate::daemon::shutdown::ShutdownMode::Graceful {
            "\u{23f3} \u{6b63}\u{5728}\u{4f18}\u{96c5}\u{5173}\u{95ed}..."
        } else {
            "\u{26a0}\u{fe0f} \u{5f3a}\u{5236}\u{5173}\u{95ed}\u{4e2d}..."
        };

        let card = json!({
            "config": { "wide_screen_mode": true },
            "header": json!({
                "title": json!({
                    "tag": "plain_text",
                    "content": header_title
                }),
                "template": if mode == crate::daemon::shutdown::ShutdownMode::Graceful { "blue" } else { "red" }
            }),
            "elements": elements
        });

        // Send one card per chat (deduplicated by chat_id).
        let plugins = self.get_all_plugins().await;
        for chat_id in chats.keys() {
            for plugin in &plugins {
                let output = RenderedOutput {
                    msg_type: "interactive".into(),
                    payload: card.clone(),
                };
                if let Err(e) = plugin.send(&output, chat_id, None).await {
                    tracing::warn!(
                        chat_id = %chat_id,
                        plugin = plugin.platform(),
                        error = %e,
                        "failed to send shutdown progress card — continuing"
                    );
                }
            }
        }
    }

    /// Send a final shutdown progress card indicating completion.
    pub async fn send_shutdown_final_card(
        &self,
        result: &crate::gateway::session_manager::stop::StopResult,
    ) {
        let sessions = self.session_manager.get_all_sessions().await;
        if sessions.is_empty() {
            return;
        }

        let mut chats: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for session in &sessions {
            if let Some(chat_id) = self.session_manager.get_chat_id(&session.id).await {
                chats.entry(chat_id).or_default().push(session.id.clone());
            }
        }
        if chats.is_empty() {
            return;
        }

        let summary = format!(
            "\u{2705} \u{5173}\u{95ed}\u{5b8c}\u{6210}\u{ff1a} {} \u{6210}\u{529f}, {} \u{5931}\u{8d25}, {} \u{8df3}\u{8fc7}",
            result.succeeded, result.failed, result.skipped
        );

        let card = json!({
            "config": { "wide_screen_mode": true },
            "header": json!({
                "title": json!({
                    "tag": "plain_text",
                    "content": "\u{2705} \u{5173}\u{95ed}\u{5b8c}\u{6210}"
                }),
                "template": "green"
            }),
            "elements": [
                json!({
                    "tag": "div",
                    "text": json!({
                        "tag": "lark_md",
                        "content": summary
                    })
                })
            ]
        });

        let plugins = self.get_all_plugins().await;
        for chat_id in chats.keys() {
            for plugin in &plugins {
                let output = RenderedOutput {
                    msg_type: "interactive".into(),
                    payload: card.clone(),
                };
                if let Err(e) = plugin.send(&output, chat_id, None).await {
                    tracing::warn!(
                        chat_id = %chat_id,
                        plugin = plugin.platform(),
                        error = %e,
                        "failed to send shutdown final card — continuing"
                    );
                }
            }
        }
    }

    /// Process an inbound message through the processor chain.
    ///
    /// Builds a [`RawMessage`] from the provided fields and runs it through
    /// the inbound processor chain (if a [`ProcessorRegistry`] is configured).
    ///
    /// - When no registry exists, returns a bypass [`ProcessedMessage`]
    ///   wrapping the original content.
    /// - On processor failure, logs a warning and falls back to the
    ///   original content.
    pub(crate) async fn process_inbound_chain(
        &self,
        platform: &str,
        sender_id: &str,
        peer_id: &str,
        content: &str,
        message_id: &str,
        timestamp_ms: i64,
    ) -> ProcessedMessage {
        let Some(registry) = &self.processor_registry else {
            return ProcessedMessage {
                content: content.to_string(),
                metadata: serde_json::Map::new(),
                suppress: false,
                content_blocks: vec![],
            };
        };

        let timestamp =
            chrono::DateTime::from_timestamp_millis(timestamp_ms).unwrap_or_else(chrono::Utc::now);

        let raw = RawMessage {
            platform: platform.to_string(),
            sender_id: sender_id.to_string(),
            peer_id: peer_id.to_string(),
            content: content.to_string(),
            timestamp,
            message_id: message_id.to_string(),
        };

        match registry.process_inbound(raw).await {
            Ok(processed) => processed,
            Err(e) => {
                tracing::warn!(?e, "processor chain failed, falling back to raw content");
                ProcessedMessage {
                    content: content.to_string(),
                    metadata: serde_json::Map::new(),
                    suppress: false,
                    content_blocks: vec![],
                }
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("Unknown channel: {0}")]
    UnknownChannel(String),

    #[error("Message too large")]
    MessageTooLarge,

    #[error("Adapter error: {0}")]
    AdapterError(String),

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Missing session ID in message metadata")]
    MissingSessionId,

    #[error("No routing key: both session_key and session_id missing from metadata")]
    NoRoutingKey,

    #[error("Outbound error: {0}")]
    OutboundError(String),
}

impl From<super::im::AdapterError> for GatewayError {
    fn from(e: super::im::AdapterError) -> Self {
        GatewayError::AdapterError(e.to_string())
    }
}

#[cfg(test)]
#[path = "priority_prompt_tests.rs"]
mod priority_prompt_tests;
#[cfg(test)]
mod session_handler_dynamic_tests;
#[cfg(test)]
mod session_handler_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_dmscope;
#[cfg(test)]
mod tests_processor_chain;
#[cfg(test)]
mod tests_thread;
