//! Gateway integration and unit tests.
//!
//! All tests live here so that src/gateway/mod.rs stays under 500 lines.

use crate::gateway::{DmScope, GatewayConfig, GatewayError, Message, SessionManager};
use crate::im::{AdapterError, IMAdapter};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

// ── Mock adapter ────────────────────────────────────────────────────────────

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

// ── Test helpers ────────────────────────────────────────────────────────────

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

fn make_gw(config: GatewayConfig) -> (crate::gateway::Gateway, Arc<SessionManager>) {
    let sm = Arc::new(SessionManager::new(&config, None));
    let gw = crate::gateway::Gateway::new(config, Arc::clone(&sm));
    (gw, sm)
}

/// Setup: gateway + session_manager + registered mock adapter.
async fn setup(
    config: GatewayConfig,
    channel: &str,
) -> (crate::gateway::Gateway, Arc<SessionManager>) {
    let (gw, sm) = make_gw(config);
    gw.register_adapter(
        channel.to_string(),
        Arc::new(MockAdapter { should_fail: false }),
    )
    .await;
    (gw, sm)
}

/// Create a message with session_id in metadata.
async fn msg_with_session(sm: &SessionManager, channel: &str, to: &str, content: &str) -> Message {
    let mut msg = make_message(to, content);
    let sid = sm.find_or_create(channel, &msg, None).await.unwrap();
    msg.metadata.insert("session_id".into(), sid);
    msg
}

/// Add session_id to an existing message.
async fn add_session(
    sm: &SessionManager,
    channel: &str,
    msg: &mut Message,
    account_id: Option<&str>,
) {
    let sid = sm.find_or_create(channel, msg, account_id).await.unwrap();
    msg.metadata.insert("session_id".into(), sid);
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

// ── Serialization tests ─────────────────────────────────────────────────────

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

// ── DmScope + compute_session_key ───────────────────────────────────────────

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
    let key = DmScope::Main.compute_session_key("ch_x", &msg("a", "b"), None);
    assert_eq!(key, "ch_x:b");
}

#[test]
fn test_dm_scope_per_peer_session_key() {
    let key = DmScope::PerPeer.compute_session_key("ch_x", &msg("a", "b"), None);
    assert_eq!(key, "a:b");
}

#[test]
fn test_dm_scope_per_channel_peer_session_key() {
    let key = DmScope::PerChannelPeer.compute_session_key("ch_x", &msg("a", "b"), None);
    assert_eq!(key, "ch_x:a:b");
}

#[test]
fn test_dm_scope_per_account_channel_peer_with_account() {
    let key =
        DmScope::PerAccountChannelPeer.compute_session_key("ch_x", &msg("a", "b"), Some("acc1"));
    assert_eq!(key, "acc1:ch_x:a:b");
}

#[test]
fn test_dm_scope_per_account_channel_peer_without_account() {
    let key = DmScope::PerAccountChannelPeer.compute_session_key("ch_x", &msg("a", "b"), None);
    assert_eq!(key, "default:ch_x:a:b");
}

#[test]
fn test_dm_scope_default_is_per_channel_peer() {
    assert_eq!(DmScope::default(), DmScope::PerChannelPeer);
}

// ── GatewayConfig serde: dm_scope values ─────────────────────────────────────

#[test]
fn test_gateway_config_dm_scope_values() {
    let cases = [
        ("\"main\"", DmScope::Main),
        ("\"per-peer\"", DmScope::PerPeer),
        ("\"per-channel-peer\"", DmScope::PerChannelPeer),
        (
            "\"per-account-channel-peer\"",
            DmScope::PerAccountChannelPeer,
        ),
    ];
    for (json_val, expected) in cases {
        let json = format!("{{\"name\":\"g\",\"dm_scope\":{}}}", json_val);
        let cfg: GatewayConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg.dm_scope, expected);
    }
    let cfg: GatewayConfig = serde_json::from_str("{\"name\":\"g\"}").unwrap();
    assert_eq!(cfg.dm_scope, DmScope::PerChannelPeer);
    assert_eq!(cfg.rate_limit_per_minute, 0);
    assert_eq!(cfg.max_message_size, 0);
}

// ── Gateway integration tests ───────────────────────────────────────────────

#[tokio::test]
async fn test_register_and_route() {
    let (gw, sm) = setup(make_config(), "mock").await;
    let msg = msg_with_session(&sm, "mock", "agent-1", "hello").await;
    gw.route_message("mock", msg, None).await.unwrap();
}

#[tokio::test]
async fn test_route_unknown_channel() {
    let (gw, sm) = setup(make_config(), "mock").await;
    let msg = msg_with_session(&sm, "mock", "agent-1", "hello").await;
    let result = gw.route_message("unknown", msg, None).await;
    assert!(matches!(result, Err(GatewayError::UnknownChannel(_))));
}

#[tokio::test]
async fn test_route_message_too_large() {
    let mut cfg = make_config();
    cfg.max_message_size = 5;
    let (gw, sm) = setup(cfg, "mock").await;
    let msg = msg_with_session(&sm, "mock", "agent-1", "this is too long").await;
    let result = gw.route_message("mock", msg, None).await;
    assert!(matches!(result, Err(GatewayError::MessageTooLarge)));
}

#[tokio::test]
async fn test_route_adapter_error() {
    let (gw, sm) = make_gw(make_config());
    gw.register_adapter("mock".into(), Arc::new(MockAdapter { should_fail: true }))
        .await;
    let msg = msg_with_session(&sm, "mock", "agent-1", "hello").await;
    let result = gw.route_message("mock", msg, None).await;
    assert!(matches!(result, Err(GatewayError::AdapterError(_))));
}

#[tokio::test]
async fn test_session_created_on_route() {
    let (gw, sm) = setup(make_config(), "mock").await;
    let msg = msg_with_session(&sm, "mock", "agent-1", "hello").await;
    gw.route_message("mock", msg, None).await.unwrap();
    let sessions = gw.get_agent_sessions("agent-1").await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].agent_id, "agent-1");
    assert_eq!(sessions[0].channel, "mock");
}

#[tokio::test]
async fn test_no_sessions_for_unknown_agent() {
    let gw = crate::gateway::Gateway::new(
        make_config(),
        Arc::new(SessionManager::new(&make_config(), None)),
    );
    assert!(gw.get_agent_sessions("nobody").await.is_empty());
}

#[tokio::test]
async fn test_session_not_duplicated() {
    let (gw, sm) = setup(make_config(), "mock").await;
    let m1 = msg_with_session(&sm, "mock", "agent-1", "first").await;
    let m2 = msg_with_session(&sm, "mock", "agent-1", "second").await;
    gw.route_message("mock", m1, None).await.unwrap();
    gw.route_message("mock", m2, None).await.unwrap();
    assert_eq!(gw.get_agent_sessions("agent-1").await.len(), 1);
}

#[tokio::test]
async fn test_route_message_no_session_id_returns_missing_session_id() {
    // When a message arrives WITHOUT session_id in metadata,
    // Gateway::route_message should return MissingSessionId.
    let (gw, _sm) = setup(make_config(), "mock").await;
    let msg_without_session = make_message("agent-1", "hello");
    let result = gw.route_message("mock", msg_without_session, None).await;
    assert!(matches!(result, Err(GatewayError::MissingSessionId)));
}

#[tokio::test]
async fn test_route_message_nonexistent_session_returns_missing_session_id() {
    // When a message arrives WITH a session_id that doesn't exist in the
    // active sessions table, Gateway::route_message should return MissingSessionId.
    let (gw, _sm) = setup(make_config(), "mock").await;
    let mut msg = make_message("agent-1", "hello");
    msg.metadata.insert(
        "session_id".to_string(),
        "nonexistent-session-id".to_string(),
    );
    let result = gw.route_message("mock", msg, None).await;
    assert!(matches!(result, Err(GatewayError::MissingSessionId)));
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

// ── Session isolation tests ─────────────────────────────────────────────────

#[tokio::test]
async fn test_per_channel_peer_different_senders() {
    let mut cfg = make_config();
    cfg.dm_scope = DmScope::PerChannelPeer;
    let (gw, sm) = setup(cfg, "ch").await;
    let mut m1 = Message {
        id: "1".into(),
        from: "alice".into(),
        to: "bob".into(),
        content: "hi".into(),
        channel: "ch".into(),
        timestamp: 0,
        metadata: HashMap::new(),
    };
    let mut m2 = Message {
        id: "2".into(),
        from: "carol".into(),
        to: "bob".into(),
        content: "hi".into(),
        channel: "ch".into(),
        timestamp: 0,
        metadata: HashMap::new(),
    };
    add_session(&sm, "ch", &mut m1, None).await;
    add_session(&sm, "ch", &mut m2, None).await;
    gw.route_message("ch", m1, None).await.unwrap();
    gw.route_message("ch", m2, None).await.unwrap();
    assert_eq!(gw.get_agent_sessions("bob").await.len(), 2);
}

#[tokio::test]
async fn test_main_scope_all_share_one_session() {
    let mut cfg = make_config();
    cfg.dm_scope = DmScope::Main;
    let (gw, sm) = setup(cfg, "ch").await;
    let mut m1 = make_message("bob", "hi");
    let mut m2 = make_message("bob", "hi");
    add_session(&sm, "ch", &mut m1, None).await;
    add_session(&sm, "ch", &mut m2, None).await;
    gw.route_message("ch", m1, None).await.unwrap();
    gw.route_message("ch", m2, None).await.unwrap();
    let sessions = gw.get_agent_sessions("bob").await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "ch:bob");
}

#[tokio::test]
async fn test_per_account_peer_different_accounts() {
    let mut cfg = make_config();
    cfg.dm_scope = DmScope::PerAccountChannelPeer;
    let (gw, sm) = setup(cfg, "ch").await;
    let mut m1 = make_message("bob", "hi");
    let mut m2 = make_message("bob", "hi");
    add_session(&sm, "ch", &mut m1, Some("acc_a")).await;
    add_session(&sm, "ch", &mut m2, Some("acc_b")).await;
    gw.route_message("ch", m1, Some("acc_a")).await.unwrap();
    gw.route_message("ch", m2, Some("acc_b")).await.unwrap();
    assert_eq!(gw.get_agent_sessions("bob").await.len(), 2);
}

#[tokio::test]
async fn test_account_id_from_metadata() {
    let mut cfg = make_config();
    cfg.dm_scope = DmScope::PerAccountChannelPeer;
    let (gw, sm) = setup(cfg, "feishu").await;
    let mut msg = make_message("agent-1", "hello");
    msg.metadata.insert("account_id".into(), "t_abc".into());
    let aid = msg.metadata.get("account_id").cloned();
    add_session(&sm, "feishu", &mut msg, aid.as_deref()).await;
    let sid = msg.metadata.get("session_id").unwrap();
    assert!(sid.starts_with("t_abc:"), "sid: {}", sid);
    gw.route_message("feishu", msg, None).await.unwrap();
    assert_eq!(gw.get_agent_sessions("agent-1").await.len(), 1);
}

#[tokio::test]
async fn test_explicit_account_id_overrides_metadata() {
    let mut cfg = make_config();
    cfg.dm_scope = DmScope::PerAccountChannelPeer;
    let (gw, sm) = setup(cfg, "feishu").await;
    let mut msg = make_message("agent-1", "hello");
    msg.metadata.insert("account_id".into(), "meta_t".into());
    add_session(&sm, "feishu", &mut msg, Some("explicit_t")).await;
    let sid = msg.metadata.get("session_id").unwrap();
    assert!(sid.starts_with("explicit_t:"), "sid: {}", sid);
    gw.route_message("feishu", msg, Some("explicit_t"))
        .await
        .unwrap();
    assert_eq!(gw.get_agent_sessions("agent-1").await.len(), 1);
}

#[tokio::test]
async fn test_feishu_session_isolation() {
    let mut cfg = make_config();
    cfg.dm_scope = DmScope::PerChannelPeer;
    let (gw, sm) = setup(cfg, "feishu").await;
    let mut m_a = make_message("agent-1", "hi");
    m_a.from = "ou_alice".into();
    let mut m_c = make_message("agent-1", "hi");
    m_c.from = "ou_carol".into();
    add_session(&sm, "feishu", &mut m_a, None).await;
    add_session(&sm, "feishu", &mut m_c, None).await;
    gw.route_message("feishu", m_a, None).await.unwrap();
    gw.route_message("feishu", m_c, None).await.unwrap();
    let sessions = gw.get_agent_sessions("agent-1").await;
    assert_eq!(sessions.len(), 2);
    let ids: Vec<_> = sessions.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.iter().any(|k| k.contains("ou_alice")));
    assert!(ids.iter().any(|k| k.contains("ou_carol")));
}
