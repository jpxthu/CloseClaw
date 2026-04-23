//! Gateway - IM protocol adapters, message routing, authentication
//!
//! Central hub that connects IM platforms (Feishu, Discord, etc.) to agents.

pub mod message;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

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
}

/// Gateway - routes messages between IM adapters and agents
pub struct Gateway {
    config: GatewayConfig,
    adapters: RwLock<HashMap<String, Arc<dyn super::im::IMAdapter>>>,
    sessions: RwLock<HashMap<String, Session>>,
}

/// Session - represents an active conversation
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub agent_id: String,
    pub channel: String,
    pub created_at: i64,
}

impl Gateway {
    /// Create a new Gateway
    pub fn new(config: GatewayConfig) -> Self {
        Self {
            config,
            adapters: RwLock::new(HashMap::new()),
            sessions: RwLock::new(HashMap::new()),
        }
    }

    /// Register an IM adapter
    pub async fn register_adapter(&self, name: String, adapter: Arc<dyn super::im::IMAdapter>) {
        let mut adapters = self.adapters.write().await;
        adapters.insert(name.clone(), adapter);
    }

    /// Route an incoming message to the appropriate agent
    pub async fn route_message(&self, channel: &str, message: Message) -> Result<(), GatewayError> {
        // Find the adapter for this channel
        let adapters = self.adapters.read().await;
        let adapter = adapters
            .get(channel)
            .ok_or(GatewayError::UnknownChannel(channel.to_string()))?;

        // Validate message size
        if message.content.len() > self.config.max_message_size {
            return Err(GatewayError::MessageTooLarge);
        }

        // Create session if needed
        let session_id = format!("{}:{}", channel, message.to);
        let mut sessions = self.sessions.write().await;
        if !sessions.contains_key(&session_id) {
            sessions.insert(
                session_id.clone(),
                Session {
                    id: session_id.clone(),
                    agent_id: message.to.clone(),
                    channel: channel.to_string(),
                    created_at: chrono::Utc::now().timestamp(),
                },
            );
        }

        // Send to adapter for delivery to agent
        adapter.send_message(&message).await?;

        Ok(())
    }

    /// Get active sessions for an agent
    pub async fn get_agent_sessions(&self, agent_id: &str) -> Vec<Session> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .filter(|s| s.agent_id == agent_id)
            .cloned()
            .collect()
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
}

impl From<super::im::AdapterError> for GatewayError {
    fn from(e: super::im::AdapterError) -> Self {
        GatewayError::AdapterError(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::im::IMAdapter;
    use async_trait::async_trait;

    /// Mock adapter that records calls
    struct MockAdapter {
        should_fail: bool,
    }

    #[async_trait]
    impl IMAdapter for MockAdapter {
        fn name(&self) -> &str {
            "mock"
        }

        async fn handle_webhook(
            &self,
            _payload: &[u8],
        ) -> Result<Message, super::super::im::AdapterError> {
            Ok(Message {
                id: "1".into(),
                from: "a".into(),
                to: "b".into(),
                content: "hi".into(),
                channel: "mock".into(),
                timestamp: 0,
                metadata: HashMap::new(),
            })
        }

        async fn send_message(
            &self,
            _message: &Message,
        ) -> Result<(), super::super::im::AdapterError> {
            if self.should_fail {
                return Err(super::super::im::AdapterError::SendFailed(
                    "mock error".into(),
                ));
            }
            Ok(())
        }

        async fn validate_signature(&self, _signature: &str, _payload: &[u8]) -> bool {
            true
        }
    }

    fn make_config() -> GatewayConfig {
        GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
        }
    }

    fn make_message(to: &str, content: &str) -> Message {
        Message {
            id: "msg_1".to_string(),
            from: "ou_sender".to_string(),
            to: to.to_string(),
            content: content.to_string(),
            channel: "mock".to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn test_gateway_config_serialization() {
        let config = make_config();
        let json = serde_json::to_string(&config).unwrap();
        let parsed: GatewayConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test");
        assert_eq!(parsed.rate_limit_per_minute, 100);
        assert_eq!(parsed.max_message_size, 1024);
    }

    #[test]
    fn test_message_serialization() {
        let msg = make_message("agent-1", "hello");
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "msg_1");
        assert_eq!(parsed.content, "hello");
    }

    #[tokio::test]
    async fn test_register_and_route() {
        let gw = Gateway::new(make_config());
        let adapter = Arc::new(MockAdapter { should_fail: false });
        gw.register_adapter("mock".to_string(), adapter).await;

        let msg = make_message("agent-1", "hello");
        let result = gw.route_message("mock", msg).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_route_unknown_channel() {
        let gw = Gateway::new(make_config());
        let msg = make_message("agent-1", "hello");
        let result = gw.route_message("unknown", msg).await;
        assert!(matches!(result, Err(GatewayError::UnknownChannel(_))));
    }

    #[tokio::test]
    async fn test_route_message_too_large() {
        let mut config = make_config();
        config.max_message_size = 5;
        let gw = Gateway::new(config);
        let adapter = Arc::new(MockAdapter { should_fail: false });
        gw.register_adapter("mock".to_string(), adapter).await;

        let msg = make_message("agent-1", "this is too long");
        let result = gw.route_message("mock", msg).await;
        assert!(matches!(result, Err(GatewayError::MessageTooLarge)));
    }

    #[tokio::test]
    async fn test_route_adapter_error() {
        let gw = Gateway::new(make_config());
        let adapter = Arc::new(MockAdapter { should_fail: true });
        gw.register_adapter("mock".to_string(), adapter).await;

        let msg = make_message("agent-1", "hello");
        let result = gw.route_message("mock", msg).await;
        assert!(matches!(result, Err(GatewayError::AdapterError(_))));
    }

    #[tokio::test]
    async fn test_session_created_on_route() {
        let gw = Gateway::new(make_config());
        let adapter = Arc::new(MockAdapter { should_fail: false });
        gw.register_adapter("mock".to_string(), adapter).await;

        let msg = make_message("agent-1", "hello");
        gw.route_message("mock", msg).await.unwrap();

        let sessions = gw.get_agent_sessions("agent-1").await;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].agent_id, "agent-1");
        assert_eq!(sessions[0].channel, "mock");
    }

    #[tokio::test]
    async fn test_no_sessions_for_unknown_agent() {
        let gw = Gateway::new(make_config());
        let sessions = gw.get_agent_sessions("nobody").await;
        assert!(sessions.is_empty());
    }

    #[tokio::test]
    async fn test_session_not_duplicated() {
        let gw = Gateway::new(make_config());
        let adapter = Arc::new(MockAdapter { should_fail: false });
        gw.register_adapter("mock".to_string(), adapter).await;

        let msg1 = make_message("agent-1", "first");
        let msg2 = make_message("agent-1", "second");
        gw.route_message("mock", msg1).await.unwrap();
        gw.route_message("mock", msg2).await.unwrap();

        let sessions = gw.get_agent_sessions("agent-1").await;
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn test_gateway_error_display() {
        assert!(GatewayError::UnknownChannel("x".into())
            .to_string()
            .contains("x"));
        assert!(GatewayError::MessageTooLarge
            .to_string()
            .contains("too large"));
        assert!(GatewayError::AdapterError("e".into())
            .to_string()
            .contains("e"));
        assert!(GatewayError::RateLimitExceeded.to_string().contains("Rate"));
    }
}
