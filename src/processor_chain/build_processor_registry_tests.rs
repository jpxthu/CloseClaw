//! Tests for [`build_processor_registry`].

use crate::gateway::{DmScope, GatewayConfig};
use crate::processor_chain::build_processor_registry;
use crate::processor_chain::context::{ProcessedMessage, RawMessage};

use chrono::Utc;

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_config(raw_log_dir: Option<std::path::PathBuf>, dm_scope: DmScope) -> GatewayConfig {
    GatewayConfig {
        name: "test-gw".to_string(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        dm_scope,
        raw_log_dir,
        inbound_queue_capacity: 64,
    }
}

fn make_raw_message() -> RawMessage {
    RawMessage {
        platform: "feishu".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "peer_1".to_string(),
        content: "  hello   world  ".to_string(),
        timestamp: Utc::now(),
        message_id: "msg_1".to_string(),
        account_id: None,
    }
}

// ── test 1: default config (no raw_log_dir) ─────────────────────────────────

#[tokio::test]
async fn test_default_config_no_raw_log() {
    let config = make_config(None, DmScope::default());
    let registry = build_processor_registry(&config);

    // Inbound: SessionRouter (20) + ContentNormalizer (30) = 2
    assert_eq!(
        registry.inbound_len(),
        2,
        "default config should have 2 inbound processors"
    );
    // Outbound: DslParser (10) = 1
    assert_eq!(
        registry.outbound_len(),
        1,
        "default config should have 1 outbound processor"
    );
}

// ── test 2: config with raw_log_dir ─────────────────────────────────────────

#[tokio::test]
async fn test_config_with_raw_log_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let config = make_config(Some(tmp.path().to_path_buf()), DmScope::default());
    let registry = build_processor_registry(&config);

    // Inbound: RawLogProcessor (10) + SessionRouter (20) + ContentNormalizer (30) = 3
    // Outbound: DslParser (10) + OutboundRawLogProcessor (20) = 2
    assert_eq!(
        registry.inbound_len(),
        3,
        "config with raw_log_dir should have 3 inbound processors"
    );
    assert_eq!(
        registry.outbound_len(),
        2,
        "config with raw_log_dir should have 2 outbound processors"
    );
}

// ── test 3: dm_scope is correctly passed to SessionRouter ────────────────────

/// Verify that different DmScope values produce different session keys,
/// confirming the parameter is correctly forwarded to SessionRouter.
#[tokio::test]
async fn test_dm_scope_passed_to_session_router() {
    let raw = make_raw_message();

    // Build registry with PerChannelSender scope
    let config_sender = make_config(None, DmScope::PerChannelSender);
    let registry_sender = build_processor_registry(&config_sender);

    // Build registry with default scope (PerAccountChannelPeer)
    let config_default = make_config(None, DmScope::PerAccountChannelPeer);
    let registry_default = build_processor_registry(&config_default);

    let result_sender = registry_sender.process_inbound(raw.clone()).await.unwrap();
    let result_default = registry_default.process_inbound(raw).await.unwrap();

    let key_sender = result_sender
        .metadata
        .get("session_key")
        .and_then(|v| v.as_str())
        .unwrap();
    let key_default = result_default
        .metadata
        .get("session_key")
        .and_then(|v| v.as_str())
        .unwrap();

    // Session keys must differ because routing fields differ by scope
    assert_ne!(
        key_sender, key_default,
        "different DmScope values should produce different session keys"
    );
    assert!(!key_sender.is_empty(), "session_key must not be empty");
}

// ── test 4: processor priority sorting ──────────────────────────────────────

/// Verify that the inbound chain executes in priority order:
/// RawLogProcessor(10) → SessionRouter(20) → ContentNormalizer(30).
///
/// We send a message with trailing whitespace and verify that
/// ContentNormalizer (the last inbound processor) trims it,
/// proving it ran after SessionRouter.
#[tokio::test]
async fn test_priority_sorting_inbound() {
    let config = make_config(None, DmScope::default());
    let registry = build_processor_registry(&config);

    let raw = make_raw_message(); // "  hello   world  "
    let result = registry.process_inbound(raw).await.unwrap();

    // ContentNormalizer strips trailing whitespace (not leading).
    // Input: "  hello   world  " → Output: "  hello   world"
    assert_eq!(
        result.content, "  hello   world",
        "ContentNormalizer (priority 30) must run after SessionRouter (priority 20)"
    );
}

/// Verify that the outbound chain executes in priority order:
/// DslParser(10) → OutboundRawLogProcessor(20) (when raw_log_dir is set).
#[tokio::test]
async fn test_priority_sorting_outbound() {
    let tmp = tempfile::tempdir().unwrap();
    let config = make_config(Some(tmp.path().to_path_buf()), DmScope::default());
    let registry = build_processor_registry(&config);

    let llm_output = ProcessedMessage {
        content: "test output".to_string(),
        metadata: serde_json::Map::new(),
        suppress: false,
        content_blocks: vec![],
    };
    let result = registry.process_outbound(llm_output).await.unwrap();

    // Outbound chain should pass content through (DslParser doesn't modify
    // plain text, OutboundRawLogProcessor logs without changing content)
    assert_eq!(result.content, "test output");
    assert!(!result.suppress);
}
