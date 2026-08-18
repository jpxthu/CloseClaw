//! Tests for bot→Agent binding resolution (Step 1.1) and
//! `/approve-once` Gateway-level interception (Step 1.2).
//!
//! Covers:
//! - Step 1.1: binding lookup hit, miss fallback, empty map, no match key
//! - Step 1.2: `/approve-once` still intercepted at Gateway level

use crate::{Gateway, GatewayConfig, HandleResult, SessionManager};
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, NormalizedMessage, RenderedOutput};
use closeclaw_common::processor::DslParseResult;
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::persistence::ReasoningLevel;
use std::collections::HashMap;
use std::sync::Arc;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_config(bindings: HashMap<String, String>) -> GatewayConfig {
    GatewayConfig {
        name: "test-binding".to_string(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        bot_agent_bindings: bindings,
        ..Default::default()
    }
}

fn make_gw(bindings: HashMap<String, String>) -> Gateway {
    let config = make_config(bindings);
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    Gateway::new(config, sm)
}

/// Shared mock plugin that captures all sent messages.
struct CapturePlugin {
    sends: std::sync::Mutex<Vec<String>>,
}

impl CapturePlugin {
    fn new() -> Self {
        Self {
            sends: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn get_sends(&self) -> Vec<String> {
        self.sends.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl IMPlugin for CapturePlugin {
    fn platform(&self) -> &str {
        "mock"
    }
    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(None)
    }
    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
        }
    }
    async fn send(
        &self,
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        let text = output.payload["content"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        self.sends.lock().unwrap().push(text);
        Ok(())
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Step 1.1: Bot→Agent Binding Resolution
// ═════════════════════════════════════════════════════════════════════════════

/// Binding hit: peer_id present in bindings returns the bound agent_id.
#[test]
fn test_resolve_agent_id_binding_hit() {
    let mut bindings = HashMap::new();
    bindings.insert("bot_x".to_string(), "agent-a".to_string());
    bindings.insert("bot_y".to_string(), "agent-b".to_string());

    assert_eq!(
        Gateway::resolve_agent_id(&bindings, "bot_x"),
        "agent-a",
        "binding lookup must return bound agent_id for bot_x"
    );
    assert_eq!(
        Gateway::resolve_agent_id(&bindings, "bot_y"),
        "agent-b",
        "binding lookup must return bound agent_id for bot_y"
    );
}

/// Binding miss: peer_id not in bindings falls back to peer_id.
#[test]
fn test_resolve_agent_id_binding_miss_fallback() {
    let mut bindings = HashMap::new();
    bindings.insert("bot_x".to_string(), "agent-a".to_string());

    assert_eq!(
        Gateway::resolve_agent_id(&bindings, "unknown_bot"),
        "unknown_bot",
        "binding miss must fallback to peer_id"
    );
}

/// Empty bindings map: all lookups fall back to peer_id.
#[test]
fn test_resolve_agent_id_empty_bindings_fallback() {
    let bindings = HashMap::new();

    assert_eq!(
        Gateway::resolve_agent_id(&bindings, "any_bot"),
        "any_bot",
        "empty bindings must fallback to peer_id"
    );
    assert_eq!(
        Gateway::resolve_agent_id(&bindings, ""),
        "",
        "empty peer_id with empty bindings must return empty string"
    );
}

/// No matching key: peer_id does not match any binding key.
#[test]
fn test_resolve_agent_id_no_matching_key() {
    let mut bindings = HashMap::new();
    bindings.insert("bot_a".to_string(), "agent-a".to_string());
    bindings.insert("bot_b".to_string(), "agent-b".to_string());

    // bot_c is not in the map — should fall back to peer_id
    assert_eq!(
        Gateway::resolve_agent_id(&bindings, "bot_c"),
        "bot_c",
        "non-matching key must fallback to peer_id"
    );
}

/// Multiple bindings: each key maps to its own agent_id.
#[test]
fn test_resolve_agent_id_multiple_bindings() {
    let mut bindings = HashMap::new();
    bindings.insert("feishu_bot".to_string(), "agent-feishu".to_string());
    bindings.insert("telegram_bot".to_string(), "agent-telegram".to_string());
    bindings.insert("slack_bot".to_string(), "agent-slack".to_string());

    assert_eq!(
        Gateway::resolve_agent_id(&bindings, "feishu_bot"),
        "agent-feishu"
    );
    assert_eq!(
        Gateway::resolve_agent_id(&bindings, "telegram_bot"),
        "agent-telegram"
    );
    assert_eq!(
        Gateway::resolve_agent_id(&bindings, "slack_bot"),
        "agent-slack"
    );
}

/// Peer_id that is also an agent_id: when not in bindings, returns itself.
#[test]
fn test_resolve_agent_id_peer_id_is_agent_id() {
    let bindings = HashMap::new();

    assert_eq!(
        Gateway::resolve_agent_id(&bindings, "agent-123"),
        "agent-123",
        "peer_id not in bindings should return peer_id unchanged"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// Step 1.2: /approve-once still intercepted at Gateway level
// ═════════════════════════════════════════════════════════════════════════════

/// When no approval_flow is configured, `/approve-once` returns None
/// (falls through to SlashDispatcher or normal handler).
#[tokio::test]
async fn test_approve_once_no_flow_falls_through() {
    let gw = make_gw(HashMap::new());

    // No approval_flow set — try_handle_approval_command returns None
    let result = gw
        .try_handle_approval_command(
            "sess1",
            "/approve-once req-123",
            Some("owner"),
            "chat_1",
            "feishu",
        )
        .await;

    assert!(
        result.is_none(),
        "/approve-once with no approval_flow should fall through"
    );
}

/// Non-owner `/approve-once` is intercepted and rejected at Gateway level.
#[tokio::test]
async fn test_approve_once_non_owner_rejected() {
    let gw = make_gw(HashMap::new());
    let plugin = Arc::new(CapturePlugin::new());
    gw.register_plugin(Arc::clone(&plugin) as Arc<dyn closeclaw_common::IMPlugin>)
        .await;

    // Non-owner attempts /approve-once — should be intercepted and rejected
    let result = gw
        .try_handle_approval_command(
            "sess1",
            "/approve-once req-123",
            Some("not_owner"),
            "chat_1",
            "mock",
        )
        .await;

    assert!(
        matches!(result, Some(HandleResult::ApprovalProcessed)),
        "non-owner /approve-once should return ApprovalProcessed"
    );

    let sends = plugin.get_sends();
    assert_eq!(sends.len(), 1, "should send one rejection message");
    assert!(
        sends[0].contains("权限不足"),
        "rejection message should mention permission denial, got: {}",
        sends[0]
    );
}

/// `/approve-whitelist` is also intercepted at Gateway level.
#[tokio::test]
async fn test_approve_whitelist_no_flow_falls_through() {
    let gw = make_gw(HashMap::new());

    let result = gw
        .try_handle_approval_command(
            "sess1",
            "/approve-whitelist req-456",
            Some("owner"),
            "chat_1",
            "feishu",
        )
        .await;

    assert!(
        result.is_none(),
        "/approve-whitelist with no approval_flow should fall through"
    );
}

/// `/deny` is intercepted at Gateway level.
#[tokio::test]
async fn test_deny_no_flow_falls_through() {
    let gw = make_gw(HashMap::new());

    let result = gw
        .try_handle_approval_command("sess1", "/deny req-789", Some("owner"), "chat_1", "feishu")
        .await;

    assert!(
        result.is_none(),
        "/deny with no approval_flow should fall through"
    );
}

/// Non-owner `/deny` is intercepted and rejected at Gateway level.
#[tokio::test]
async fn test_deny_non_owner_rejected() {
    let gw = make_gw(HashMap::new());
    let plugin = Arc::new(CapturePlugin::new());
    gw.register_plugin(Arc::clone(&plugin) as Arc<dyn closeclaw_common::IMPlugin>)
        .await;

    let result = gw
        .try_handle_approval_command(
            "sess1",
            "/deny req-789",
            Some("not_owner"),
            "chat_1",
            "mock",
        )
        .await;

    assert!(
        matches!(result, Some(HandleResult::ApprovalProcessed)),
        "non-owner /deny should return ApprovalProcessed"
    );

    let sends = plugin.get_sends();
    assert_eq!(sends.len(), 1, "should send one rejection message");
    assert!(
        sends[0].contains("权限不足"),
        "rejection message should mention permission denial"
    );
}

/// `/approve-once` without request_id returns None (malformed command).
#[tokio::test]
async fn test_approve_once_missing_request_id() {
    let gw = make_gw(HashMap::new());

    let result = gw
        .try_handle_approval_command("sess1", "/approve-once", Some("owner"), "chat_1", "feishu")
        .await;

    assert!(
        result.is_none(),
        "/approve-once without request_id should fall through"
    );
}
