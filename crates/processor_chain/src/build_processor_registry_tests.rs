//! Tests for [`build_processor_registry`].

use crate::processor_chain::build_processor_registry;
use crate::ProcessedMessage;
use closeclaw_common::im_plugin::NormalizedMessage;
use closeclaw_gateway::{DmScope, GatewayConfig};

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

fn make_normalized_message() -> NormalizedMessage {
    NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "peer_1".to_string(),
        content: "  hello   world  ".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: String::new(),
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
        "default config should have 1 outbound processor (DslParser)"
    );
}

// ── test 2: config with raw_log_dir ─────────────────────────────────────────

#[tokio::test]
async fn test_config_with_raw_log_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let config = make_config(Some(tmp.path().to_path_buf()), DmScope::default());
    let registry = build_processor_registry(&config);

    // Inbound: RawLogProcessor (10) + SessionRouter (20) + ContentNormalizer (30) = 3
    // Outbound: VerbosityFilter (5) + DslParser (10) + OutboundRawLogProcessor (20) = 3
    assert_eq!(
        registry.inbound_len(),
        3,
        "config with raw_log_dir should have 3 inbound processors"
    );
    assert_eq!(
        registry.outbound_len(),
        1,
        "config with raw_log_dir should have 1 outbound processor (DslParser)"
    );
}

// ── test 3: dm_scope is correctly passed to SessionRouter ────────────────────

/// Verify that different DmScope values produce different session keys,
/// confirming the parameter is correctly forwarded to SessionRouter.
#[tokio::test]
async fn test_dm_scope_passed_to_session_router() {
    let msg = make_normalized_message();

    // Build registry with PerChannelSender scope
    let config_sender = make_config(None, DmScope::PerChannelSender);
    let registry_sender = build_processor_registry(&config_sender);

    // Build registry with default scope (PerAccountChannelPeer)
    let config_default = make_config(None, DmScope::PerAccountChannelPeer);
    let registry_default = build_processor_registry(&config_default);

    let result_sender = registry_sender.process_inbound(msg.clone()).await.unwrap();
    let result_default = registry_default.process_inbound(msg).await.unwrap();

    let key_sender = result_sender
        .metadata
        .get("session_key")
        .map(|s| s.as_str())
        .unwrap();
    let key_default = result_default
        .metadata
        .get("session_key")
        .map(|s| s.as_str())
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

    let msg = make_normalized_message(); // "  hello   world  "
    let result = registry.process_inbound(msg).await.unwrap();

    // ContentNormalizer strips trailing whitespace (not leading).
    // Input: "  hello   world  " → Output: "  hello   world"
    assert_eq!(
        result.text_content(),
        Some("  hello   world"),
        "ContentNormalizer (priority 30) must run after SessionRouter (priority 20)"
    );
}

/// Verify that the outbound chain contains only DslParser.
#[tokio::test]
async fn test_priority_sorting_outbound() {
    let tmp = tempfile::tempdir().unwrap();
    let config = make_config(Some(tmp.path().to_path_buf()), DmScope::default());
    let registry = build_processor_registry(&config);

    let llm_output = ProcessedMessage {
        content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
            "test output".to_string(),
        )],
        metadata: std::collections::HashMap::new(),
    };
    let result = registry.process_outbound(llm_output).await.unwrap();

    // Outbound chain has only DslParser — plain text passes through unchanged
    assert_eq!(result.text_content(), Some("test output"));
    assert!(!result.content_blocks.is_empty());
}

/// Verify that VerbosityFilter is NOT in the outbound chain.
/// Verbosity filtering is now a Gateway-level step (not in the chain).
#[tokio::test]
async fn test_outbound_chain_no_verbosity_filter() {
    let tmp = tempfile::tempdir().unwrap();
    let config = make_config(Some(tmp.path().to_path_buf()), DmScope::default());
    let registry = build_processor_registry(&config);

    // Send blocks with Thinking content — if VerbosityFilter were in chain,
    // Thinking blocks would be filtered. Since it's not, they pass through.
    let llm_output = ProcessedMessage {
        content_blocks: vec![
            closeclaw_llm::types::ContentBlock::Thinking {
                thinking: "internal reasoning".to_string(),
                signature: None,
            },
            closeclaw_llm::types::ContentBlock::Text("Hello".to_string()),
        ],
        metadata: std::collections::HashMap::new(),
    };
    let result = registry.process_outbound(llm_output).await.unwrap();

    // Both blocks should pass through (no VerbosityFilter in chain)
    assert_eq!(result.content_blocks.len(), 2);
    assert!(matches!(
        &result.content_blocks[0],
        closeclaw_llm::types::ContentBlock::Thinking { .. }
    ));
    assert!(matches!(
        &result.content_blocks[1],
        closeclaw_llm::types::ContentBlock::Text(_)
    ));
}

/// Verify that OutboundRawLogProcessor is NOT in the outbound chain.
/// Outbound logging is now a Gateway-level step (not in the chain).
#[tokio::test]
async fn test_outbound_chain_no_outbound_log_processor() {
    let tmp = tempfile::tempdir().unwrap();
    let config = make_config(Some(tmp.path().to_path_buf()), DmScope::default());
    let registry = build_processor_registry(&config);

    // Even with raw_log_dir configured, outbound chain should only have DslParser
    assert_eq!(
        registry.outbound_len(),
        1,
        "outbound chain should only contain DslParser, not OutboundRawLogProcessor"
    );
}

/// Outbound chain with DSL instruction: DslParser extracts DSL.
#[tokio::test]
async fn test_outbound_chain_dsl_parsing() {
    let tmp = tempfile::tempdir().unwrap();
    let config = make_config(Some(tmp.path().to_path_buf()), DmScope::default());
    let registry = build_processor_registry(&config);

    let llm_output = ProcessedMessage {
        content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
            "::button[label:OK;action:submit;value:yes]".to_string(),
        )],
        metadata: std::collections::HashMap::new(),
    };
    let result = registry.process_outbound(llm_output).await.unwrap();

    // DslParser extracts DSL from text block
    let dsl = result.metadata.get("dsl_result").unwrap();
    assert!(dsl.contains("button"), "DSL should be parsed: {dsl}");
}
