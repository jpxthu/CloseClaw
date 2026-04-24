//! Gateway integration and unit tests.
//!
//! All tests live here so that src/gateway/mod.rs stays under 500 lines.

use crate::gateway::{DmScope, GatewayConfig, GatewayError, Message};
use crate::im::{AdapterError, IMAdapter};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

// Mock adapter

struct MockAdapter {
    should_fail: bool,
}

#[async_trait]
impl IMAdapter for MockAdapter {
    fn name(&self) -> &str {
        "mock"
    }

    async fn handle_webhook(&self, _payload: &[u8]) -> Result<Message, AdapterError> {
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

    async fn send_message(&self, _message: &Message) -> Result<(), AdapterError> {
        if self.should_fail {
            return Err(AdapterError::SendFailed("mock error".into()));
        }
        Ok(())
    }

    async fn validate_signature(&self, _signature: &str, _payload: &[u8]) -> bool {
        true
    }
}

// Test helpers

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: DmScope::default(),
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

fn feishu_msg(from: &str, to: &str) -> Message {
    Message {
        id: "x".into(),
        from: from.into(),
        to: to.into(),
        content: "hi".into(),
        channel: "feishu".into(),
        timestamp: 0,
        metadata: HashMap::new(),
    }
}

// Serialisation tests

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

// DmScope + compute_session_key unit tests

fn msg(from: &str, to: &str) -> Message {
    Message {
        id: "m".into(),
        from: from.into(),
        to: to.into(),
        content: "hi".into(),
        channel: "ch".into(),
        timestamp: 0,
        metadata: HashMap::new(),
    }
}

#[test]
fn test_dm_scope_main_session_key() {
    let scope = DmScope::Main;
    let m = msg("alice", "bob");
    // Main → channel:agent_id (agent_id = message.to = "bob")
    let key = scope.compute_session_key("channel_x", &m, None);
    assert_eq!(key, "channel_x:bob");
}

#[test]
fn test_dm_scope_per_peer_session_key() {
    let scope = DmScope::PerPeer;
    let m = msg("alice", "bob");
    let key = scope.compute_session_key("channel_x", &m, None);
    assert_eq!(key, "alice:bob");
}

#[test]
fn test_dm_scope_per_channel_peer_session_key() {
    let scope = DmScope::PerChannelPeer;
    let m = msg("alice", "bob");
    let key = scope.compute_session_key("channel_x", &m, None);
    assert_eq!(key, "channel_x:alice:bob");
}

#[test]
fn test_dm_scope_per_account_channel_peer_with_account() {
    let scope = DmScope::PerAccountChannelPeer;
    let m = msg("alice", "bob");
    let key = scope.compute_session_key("channel_x", &m, Some("acc_tenant1"));
    assert_eq!(key, "acc_tenant1:channel_x:alice:bob");
}

#[test]
fn test_dm_scope_per_account_channel_peer_without_account() {
    let scope = DmScope::PerAccountChannelPeer;
    let m = msg("alice", "bob");
    let key = scope.compute_session_key("channel_x", &m, None);
    // account part falls back to "default"
    assert_eq!(key, "default:channel_x:alice:bob");
}

#[test]
fn test_dm_scope_default_is_per_channel_peer() {
    assert_eq!(DmScope::default(), DmScope::PerChannelPeer);
}

// GatewayConfig serde: dm_scope values

#[test]
fn test_gateway_config_dm_scope_main() {
    let json = r#"{"name":"g","dm_scope":"main"}"#;
    let cfg: GatewayConfig = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.dm_scope, DmScope::Main);
}

#[test]
fn test_gateway_config_dm_scope_per_peer() {
    let json = r#"{"name":"g","dm_scope":"per-peer"}"#;
    let cfg: GatewayConfig = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.dm_scope, DmScope::PerPeer);
}

#[test]
fn test_gateway_config_dm_scope_per_channel_peer() {
    let json = r#"{"name":"g","dm_scope":"per-channel-peer"}"#;
    let cfg: GatewayConfig = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.dm_scope, DmScope::PerChannelPeer);
}

#[test]
fn test_gateway_config_dm_scope_per_account_channel_peer() {
    let json = r#"{"name":"g","dm_scope":"per-account-channel-peer"}"#;
    let cfg: GatewayConfig = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.dm_scope, DmScope::PerAccountChannelPeer);
}

#[test]
fn test_gateway_config_dm_scope_defaults_when_missing() {
    let json = r#"{"name":"g"}"#;
    let cfg: GatewayConfig = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.dm_scope, DmScope::PerChannelPeer);
    assert_eq!(cfg.rate_limit_per_minute, 0);
    assert_eq!(cfg.max_message_size, 0);
}

// Gateway integration tests

#[tokio::test]
async fn test_register_and_route() {
    let gw = crate::gateway::Gateway::new(make_config());
    let adapter = Arc::new(MockAdapter { should_fail: false });
    gw.register_adapter("mock".to_string(), adapter).await;
    let msg = make_message("agent-1", "hello");
    let result = gw.route_message("mock", msg, None).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_route_unknown_channel() {
    let gw = crate::gateway::Gateway::new(make_config());
    let msg = make_message("agent-1", "hello");
    let result = gw.route_message("unknown", msg, None).await;
    assert!(matches!(result, Err(GatewayError::UnknownChannel(_))));
}

#[tokio::test]
async fn test_route_message_too_large() {
    let mut config = make_config();
    config.max_message_size = 5;
    let gw = crate::gateway::Gateway::new(config);
    let adapter = Arc::new(MockAdapter { should_fail: false });
    gw.register_adapter("mock".to_string(), adapter).await;
    let msg = make_message("agent-1", "this is too long");
    let result = gw.route_message("mock", msg, None).await;
    assert!(matches!(result, Err(GatewayError::MessageTooLarge)));
}

#[tokio::test]
async fn test_route_adapter_error() {
    let gw = crate::gateway::Gateway::new(make_config());
    let adapter = Arc::new(MockAdapter { should_fail: true });
    gw.register_adapter("mock".to_string(), adapter).await;
    let msg = make_message("agent-1", "hello");
    let result = gw.route_message("mock", msg, None).await;
    assert!(matches!(result, Err(GatewayError::AdapterError(_))));
}

#[tokio::test]
async fn test_session_created_on_route() {
    let gw = crate::gateway::Gateway::new(make_config());
    let adapter = Arc::new(MockAdapter { should_fail: false });
    gw.register_adapter("mock".to_string(), adapter).await;
    let msg = make_message("agent-1", "hello");
    gw.route_message("mock", msg, None).await.unwrap();
    let sessions = gw.get_agent_sessions("agent-1").await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].agent_id, "agent-1");
    assert_eq!(sessions[0].channel, "mock");
}

#[tokio::test]
async fn test_no_sessions_for_unknown_agent() {
    let gw = crate::gateway::Gateway::new(make_config());
    let sessions = gw.get_agent_sessions("nobody").await;
    assert!(sessions.is_empty());
}

#[tokio::test]
async fn test_session_not_duplicated() {
    let gw = crate::gateway::Gateway::new(make_config());
    let adapter = Arc::new(MockAdapter { should_fail: false });
    gw.register_adapter("mock".to_string(), adapter).await;
    let msg1 = make_message("agent-1", "first");
    let msg2 = make_message("agent-1", "second");
    gw.route_message("mock", msg1, None).await.unwrap();
    gw.route_message("mock", msg2, None).await.unwrap();
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

// Session isolation tests

#[tokio::test]
async fn test_per_channel_peer_different_senders_different_sessions() {
    let mut cfg = make_config();
    cfg.dm_scope = DmScope::PerChannelPeer;
    let gw = crate::gateway::Gateway::new(cfg);
    let adapter = Arc::new(MockAdapter { should_fail: false });
    gw.register_adapter("ch".to_string(), adapter).await;
    let m1 = Message {
        id: "1".into(),
        from: "alice".into(),
        to: "bob".into(),
        content: "hi".into(),
        channel: "ch".into(),
        timestamp: 0,
        metadata: HashMap::new(),
    };
    let m2 = Message {
        id: "2".into(),
        from: "carol".into(),
        to: "bob".into(),
        content: "hi".into(),
        channel: "ch".into(),
        timestamp: 0,
        metadata: HashMap::new(),
    };
    gw.route_message("ch", m1, None).await.unwrap();
    gw.route_message("ch", m2, None).await.unwrap();
    let sessions = gw.get_agent_sessions("bob").await;
    assert_eq!(sessions.len(), 2);
}

#[tokio::test]
async fn test_main_scope_all_messages_share_one_session() {
    let mut cfg = make_config();
    cfg.dm_scope = DmScope::Main;
    let gw = crate::gateway::Gateway::new(cfg);
    let adapter = Arc::new(MockAdapter { should_fail: false });
    gw.register_adapter("ch".to_string(), adapter).await;
    let m1 = make_message("bob", "hi");
    let m2 = make_message("bob", "hi");
    gw.route_message("ch", m1, None).await.unwrap();
    gw.route_message("ch", m2, None).await.unwrap();
    let sessions = gw.get_agent_sessions("bob").await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "ch:bob");
}

#[tokio::test]
async fn test_per_account_channel_peer_different_accounts_different_sessions() {
    let mut cfg = make_config();
    cfg.dm_scope = DmScope::PerAccountChannelPeer;
    let gw = crate::gateway::Gateway::new(cfg);
    let adapter = Arc::new(MockAdapter { should_fail: false });
    gw.register_adapter("ch".to_string(), adapter).await;
    let m1 = make_message("bob", "hi");
    let m2 = make_message("bob", "hi");
    gw.route_message("ch", m1, Some("acc_a")).await.unwrap();
    gw.route_message("ch", m2, Some("acc_b")).await.unwrap();
    let sessions = gw.get_agent_sessions("bob").await;
    assert_eq!(sessions.len(), 2);
}

// account_id metadata extraction tests

#[tokio::test]
async fn test_route_message_extracts_account_id_from_metadata() {
    let mut cfg = make_config();
    cfg.dm_scope = DmScope::PerAccountChannelPeer;
    let gw = crate::gateway::Gateway::new(cfg);
    let adapter = Arc::new(MockAdapter { should_fail: false });
    gw.register_adapter("feishu".to_string(), adapter).await;

    let mut msg = make_message("agent-1", "hello");
    msg.metadata
        .insert("account_id".to_string(), "tenant_abc".to_string());

    // Pass None for account_id — should fall back to metadata
    gw.route_message("feishu", msg, None).await.unwrap();

    let sessions = gw.get_agent_sessions("agent-1").await;
    assert_eq!(sessions.len(), 1);
    // Session key should include the account_id from metadata via PerAccountChannelPeer
    assert!(sessions[0].id.starts_with("tenant_abc:"));
}

#[tokio::test]
async fn test_route_message_explicit_account_id_overrides_metadata() {
    let mut cfg = make_config();
    cfg.dm_scope = DmScope::PerAccountChannelPeer;
    let gw = crate::gateway::Gateway::new(cfg);
    let adapter = Arc::new(MockAdapter { should_fail: false });
    gw.register_adapter("feishu".to_string(), adapter).await;

    let mut msg = make_message("agent-1", "hello");
    msg.metadata
        .insert("account_id".to_string(), "metadata_tenant".to_string());

    // Explicit account_id should take precedence over metadata
    gw.route_message("feishu", msg, Some("explicit_tenant"))
        .await
        .unwrap();

    let sessions = gw.get_agent_sessions("agent-1").await;
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].id.starts_with("explicit_tenant:"));
}

#[tokio::test]
async fn test_per_channel_peer_feishu_session_isolation() {
    // Simulates Feishu DM scenario: two different users (different `from` open_ids)
    // messaging the same agent on the same channel — must get different sessions.
    let mut cfg = make_config();
    cfg.dm_scope = DmScope::PerChannelPeer;
    let gw = crate::gateway::Gateway::new(cfg);
    let adapter = Arc::new(MockAdapter { should_fail: false });
    gw.register_adapter("feishu".to_string(), adapter).await;

    let mut msg_alice = make_message("agent-1", "hello from alice");
    msg_alice.from = "ou_alice".to_string();

    let mut msg_carol = make_message("agent-1", "hello from carol");
    msg_carol.from = "ou_carol".to_string();

    gw.route_message("feishu", msg_alice, None).await.unwrap();
    gw.route_message("feishu", msg_carol, None).await.unwrap();

    let sessions = gw.get_agent_sessions("agent-1").await;
    assert_eq!(sessions.len(), 2);
    let ids: Vec<_> = sessions.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.iter().any(|k| k.contains("ou_alice")));
    assert!(ids.iter().any(|k| k.contains("ou_carol")));
}

// DmScope Feishu isolation tests

// Combined test: per_channel_peer isolation, main shared session, per_account_channel_peer multi-tenant
#[tokio::test]
async fn test_feishu_dm_scope_isolation_variants() {
    // Variant 1: PerChannelPeer — different open_ids → different sessions
    {
        let mut cfg = make_config();
        cfg.dm_scope = DmScope::PerChannelPeer;
        let gw = crate::gateway::Gateway::new(cfg);
        let adapter = Arc::new(MockAdapter { should_fail: false });
        gw.register_adapter("feishu".to_string(), adapter).await;
        let m1 = feishu_msg("ou_u1", "ag");
        let m2 = feishu_msg("ou_u2", "ag");
        gw.route_message("feishu", m1, None).await.unwrap();
        gw.route_message("feishu", m2, None).await.unwrap();
        let sessions = gw.get_agent_sessions("ag").await;
        assert_eq!(sessions.len(), 2);
        assert_ne!(sessions[0].id, sessions[1].id);
    }
    // Variant 2: Main — all users share one session
    {
        let mut cfg = make_config();
        cfg.dm_scope = DmScope::Main;
        let gw = crate::gateway::Gateway::new(cfg);
        let adapter = Arc::new(MockAdapter { should_fail: false });
        gw.register_adapter("feishu".to_string(), adapter).await;
        let m1 = feishu_msg("ou_u1", "ag");
        let m2 = feishu_msg("ou_u2", "ag");
        gw.route_message("feishu", m1, None).await.unwrap();
        gw.route_message("feishu", m2, None).await.unwrap();
        let sessions = gw.get_agent_sessions("ag").await;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "feishu:ag");
    }
    // Variant 3: PerAccountChannelPeer — different tenants → different sessions
    {
        let mut cfg = make_config();
        cfg.dm_scope = DmScope::PerAccountChannelPeer;
        let gw = crate::gateway::Gateway::new(cfg);
        let adapter = Arc::new(MockAdapter { should_fail: false });
        gw.register_adapter("feishu".to_string(), adapter).await;
        let mut m1 = feishu_msg("ou_u1", "ag");
        m1.metadata.insert("account_id".into(), "tenant_a".into());
        let mut m2 = feishu_msg("ou_u1", "ag");
        m2.metadata.insert("account_id".into(), "tenant_b".into());
        gw.route_message("feishu", m1, None).await.unwrap();
        gw.route_message("feishu", m2, None).await.unwrap();
        let sessions = gw.get_agent_sessions("ag").await;
        assert_eq!(sessions.len(), 2);
        let ids: Vec<_> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.iter().any(|k| k.starts_with("tenant_a:")));
        assert!(ids.iter().any(|k| k.starts_with("tenant_b:")));
    }
}
