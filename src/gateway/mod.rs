//! Gateway - IM protocol adapters, message routing, authentication
//!
//! Central hub that connects IM platforms (Feishu, Discord, etc.) to agents.

pub mod approval;
pub mod message;
pub mod outbound;
pub mod session_handler;
pub mod session_manager;
pub mod system_prompt_inject;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::permission::approval_flow::ApprovalFlow;
use crate::session::checkpoint_manager::CheckpointManager;
use crate::session::persistence::PersistenceService;

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
}

/// Session - represents an active conversation
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub agent_id: String,
    pub channel: String,
    pub created_at: i64,
}

/// Gateway - routes messages between IM adapters and agents
pub struct Gateway {
    config: GatewayConfig,
    adapters: RwLock<HashMap<String, Arc<dyn super::im::IMAdapter>>>,
    session_manager: Arc<SessionManager>,
    processor_registry: Option<Arc<ProcessorRegistry>>,
    renderer: Option<Arc<dyn crate::renderer::Renderer>>,
    checkpoint_manager: Option<Arc<CheckpointManager<dyn PersistenceService>>>,
    session_handler: Option<Arc<SessionMessageHandler>>,
    /// Daemon-level approval flow for intercepting `/approve` / `/deny` commands.
    approval_flow: RwLock<Option<Arc<tokio::sync::Mutex<ApprovalFlow>>>>,
}

impl Gateway {
    /// Create a new Gateway with the given config and a shared SessionManager.
    pub fn new(config: GatewayConfig, session_manager: Arc<SessionManager>) -> Self {
        Self {
            config,
            adapters: RwLock::new(HashMap::new()),
            session_manager,
            processor_registry: None,
            renderer: None,
            checkpoint_manager: None,
            session_handler: None,
            approval_flow: RwLock::new(None),
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
            adapters: RwLock::new(HashMap::new()),
            session_manager,
            processor_registry: Some(registry),
            renderer: None,
            checkpoint_manager: None,
            session_handler: None,
            approval_flow: RwLock::new(None),
        }
    }

    /// Create a new Gateway with a renderer.
    ///
    /// The renderer is used in `send_outbound` to render LLM output to
    /// platform-specific formats (card or text). Optionally takes a
    /// processor registry for preprocessing.
    pub fn with_renderer(
        config: GatewayConfig,
        session_manager: Arc<SessionManager>,
        renderer: Arc<dyn crate::renderer::Renderer>,
        registry: Option<Arc<ProcessorRegistry>>,
    ) -> Self {
        Self {
            config,
            adapters: RwLock::new(HashMap::new()),
            session_manager,
            processor_registry: registry,
            renderer: Some(renderer),
            checkpoint_manager: None,
            session_handler: None,
            approval_flow: RwLock::new(None),
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

    /// Handle an inbound message through the busy/pending state machine.
    ///
    /// If `sender_id` is provided and the message starts with `/approve` or
    /// `/deny`, the approval flow intercepts the command (owner-only).
    ///
    /// Returns `HandleResult` (`LlmStarted`/`MessageQueued`/`ApprovalProcessed`),
    /// or `None` if no handler configured.
    pub async fn handle_inbound_message(
        &self,
        session_id: &str,
        content: String,
        sender_id: Option<&str>,
    ) -> Option<HandleResult> {
        // ── Approval command interception ──────────────────────────────
        if let Some(result) = self
            .try_handle_approval_command(session_id, &content, sender_id)
            .await
        {
            return Some(result);
        }

        let handler = self.session_handler.as_ref()?;
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

    /// Register an IM adapter.
    pub async fn register_adapter(&self, name: String, adapter: Arc<dyn super::im::IMAdapter>) {
        let mut adapters = self.adapters.write().await;
        adapters.insert(name, adapter);
    }

    /// Route an incoming message to the appropriate agent.
    ///
    /// Reads `session_id` from `message.metadata`. Returns `MissingSessionId`
    /// if absent. Validates the session exists in the active sessions table
    /// before forwarding to the adapter.
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

        // Find the adapter for this channel.
        let adapters = self.adapters.read().await;
        let adapter = adapters
            .get(channel)
            .ok_or(GatewayError::UnknownChannel(channel.to_string()))?;

        // Validate message size.
        if message.content.len() > self.config.max_message_size {
            return Err(GatewayError::MessageTooLarge);
        }

        // Forward to adapter for delivery.
        adapter.send_message(&message).await?;

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
mod session_handler_tests;
mod tests;
mod tests_dmscope;
