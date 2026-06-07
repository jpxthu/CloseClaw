//! Gateway - IM protocol adapters, message routing, authentication
//!
//! Central hub that connects IM platforms (Feishu, Discord, etc.) to agents.

pub mod approval;
pub mod message;
pub mod outbound;
pub mod session_handler;
mod session_handler_announce;
mod session_handler_dispatch;
mod session_handler_streaming;
pub mod session_manager;
pub mod slash_permission;
pub mod system_prompt_inject;

#[cfg(test)]
mod tests_plugin;
#[cfg(test)]
mod tests_slash_permission;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::permission::approval_flow::ApprovalFlow;
use crate::permission::engine::engine_eval::PermissionEngine;
use crate::renderer::RenderedOutput;
use crate::session::checkpoint_manager::CheckpointManager;
use crate::session::persistence::PersistenceService;
use crate::slash::SlashDispatcher;

pub use crate::processor_chain::ProcessorRegistry;
pub use session_handler::{HandleResult, SessionMessageHandler};
pub use session_manager::SessionManager;

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
}

#[allow(clippy::derivable_impls)]
impl Default for DmScope {
    fn default() -> Self {
        DmScope::PerChannelPeer
    }
}

impl DmScope {
    /// Compute a session key for the given context.
    pub fn compute_session_key(
        &self,
        channel: &str,
        message: &Message,
        account_id: Option<&str>,
    ) -> String {
        match self {
            DmScope::Main => format!("{}:{}", channel, message.to),
            DmScope::PerPeer => format!("{}:{}", message.from, message.to),
            DmScope::PerChannelPeer => {
                format!("{}:{}:{}", channel, message.from, message.to)
            }
            DmScope::PerAccountChannelPeer => {
                let acc = account_id.unwrap_or("default");
                format!("{}:{}:{}:{}", acc, channel, message.from, message.to)
            }
        }
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
    /// Self-reference for back-pointer to the owning `Arc<Gateway>`.
    ///
    /// `handle_inbound_message` is called with `&self`, but
    /// `SessionMessageHandler` needs an `Arc<Gateway>` to call
    /// `send_outbound_streaming`. The caller wires this after wrapping
    /// the `Gateway` in `Arc::new(...)` via `set_self_ref`. Until set,
    /// the slot is `None`; the handler falls back to the non-streaming
    /// path in that case.
    self_ref: std::sync::Mutex<Option<Arc<Gateway>>>,
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
            self_ref: std::sync::Mutex::new(None),
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
            self_ref: std::sync::Mutex::new(None),
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

    /// Handle an inbound message through the busy/pending state machine.
    ///
    /// If `sender_id` is provided and the message starts with `/approve` or
    /// `/deny`, the approval flow intercepts the command (owner-only).
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
        session_id: &str,
        content: String,
        sender_id: Option<&str>,
        channel: &str,
    ) -> Option<HandleResult> {
        // ── Approval command interception ──────────────────────────────
        if let Some(result) = self
            .try_handle_approval_command(session_id, &content, sender_id)
            .await
        {
            return Some(result);
        }

        // ── Slash command dispatch ─────────────────────────────────────
        if content.starts_with('/') {
            if let Some(result) = self
                .dispatch_slash(session_id, &content, sender_id, channel)
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
            return Some(
                handler
                    .handle_message_with_gateway(session_id, content, meta, &gw, &plugin)
                    .await,
            );
        }

        Some(handler.handle_message(session_id, content).await)
    }

    /// Configure the persistence storage backend (proxied to SessionManager).
    pub async fn set_storage(&self, storage: Arc<dyn PersistenceService>) {
        self.session_manager.set_storage(storage).await;
    }

    /// Flush all active sessions to persistence (proxied to SessionManager).
    pub async fn flush_all_sessions(
        &self,
    ) -> Result<usize, crate::session::persistence::PersistenceError> {
        self.session_manager.flush_all().await
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

    /// Get a registered IM plugin by platform identifier.
    pub async fn get_plugin(&self, platform: &str) -> Option<Arc<dyn super::im::IMPlugin>> {
        let plugins = self.plugins.read().await;
        plugins.get(platform).cloned()
    }

    /// Route an incoming message to the appropriate agent.
    ///
    /// Reads `session_id` from `message.metadata`. Returns `MissingSessionId`
    /// if absent. Validates the session exists in the active sessions table
    /// before forwarding to the registered IM plugin.
    pub async fn route_message(
        &self,
        channel: &str,
        message: Message,
        _account_id: Option<&str>,
    ) -> Result<(), GatewayError> {
        // Read session_id from metadata (written there by SessionRouter).
        let session_id = message
            .metadata
            .get("session_id")
            .ok_or(GatewayError::MissingSessionId)?;

        // Verify session exists in the active sessions table.
        if !self.session_manager.has_session(session_id).await {
            return Err(GatewayError::MissingSessionId);
        }

        // Find the IM plugin registered for this channel.
        let plugin = self
            .get_plugin(channel)
            .await
            .ok_or(GatewayError::UnknownChannel(channel.to_string()))?;

        // Validate message size.
        if message.content.len() > self.config.max_message_size {
            return Err(GatewayError::MessageTooLarge);
        }

        // Resolve thread_id from the session checkpoint.
        let thread_id = self.session_manager.get_thread_id(session_id).await;

        // Build a text RenderedOutput and forward to the plugin.
        let output = RenderedOutput {
            msg_type: "text".into(),
            payload: json!({"content": {"text": message.content}}),
        };
        plugin
            .send(&output, &message.to, thread_id.as_deref())
            .await?;

        Ok(())
    }

    /// Get active sessions for an agent (proxied to SessionManager).
    pub async fn get_agent_sessions(&self, agent_id: &str) -> Vec<Session> {
        self.session_manager.get_agent_sessions(agent_id).await
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
mod tests_thread;
