//! Gateway - IM protocol adapters, message routing, authentication
//!
//! Central hub that connects IM platforms (Feishu, Discord, etc.) to agents.

pub mod message;
pub mod session_manager;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::session::persistence::PersistenceService;

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
}

impl Gateway {
    /// Create a new Gateway with the given config and a shared SessionManager.
    pub fn new(config: GatewayConfig, session_manager: Arc<SessionManager>) -> Self {
        Self {
            config,
            adapters: RwLock::new(HashMap::new()),
            session_manager,
        }
    }

    /// Configure the persistence storage backend (proxied to SessionManager).
    pub async fn set_storage(&self, storage: Arc<dyn PersistenceService>) {
        self.session_manager.set_storage(storage).await;
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
}

impl From<super::im::AdapterError> for GatewayError {
    fn from(e: super::im::AdapterError) -> Self {
        GatewayError::AdapterError(e.to_string())
    }
}

#[cfg(test)]
mod tests;
mod tests_archive;
mod tests_dmscope;
