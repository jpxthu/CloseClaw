//! IM inbound chain E2E tests — full ProcessorRegistry with real fixtures.
//!
//! Covers:
//! - p2p DM: raw webhook → SessionRouter → FeishuMessageCleaner → plain-text output
//! - group chat: rejected by SessionRouter with `SessionNotSupportedForChannel`
//!
//! Run with: `cargo test --test im_inbound_e2e_tests`

use std::path::PathBuf;
use std::sync::Arc;

use closeclaw::gateway::{DmScope, GatewayConfig, SessionManager};
use closeclaw::im::processor::{ProcessError, ProcessorRegistry};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_session_manager() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        &GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 65536,
            dm_scope: DmScope::PerChannelPeer,
        },
        None,
    ))
}

fn feishu_fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/feishu")
}

fn load_raw_fixture(filename: &str) -> serde_json::Value {
    let path = feishu_fixtures_dir().join(filename);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

// ---------------------------------------------------------------------------
// Test 1 — p2p DM full chain
// ---------------------------------------------------------------------------

/// Verifies the full p2p DM inbound chain:
/// raw feishu webhook → SessionRouter (prio 20) → FeishuMessageCleaner (prio 30)
/// produces plain-text content and populates session metadata.
#[tokio::test]
async fn test_p2p_dm_full_chain() {
    let mgr = test_session_manager();
    let registry = ProcessorRegistry::new(mgr);
    let raw = load_raw_fixture("im-message-receive_v1-no-event-id-2026-04-26T18-53-09-967Z.json");

    let result = registry
        .process_inbound(&raw)
        .await
        .expect("p2p chain should succeed");

    // FeishuMessageCleaner extracts plain text from {"text":"..."} JSON content.
    assert!(
        !result.content.starts_with('{'),
        "content should be plain text, got: {}",
        result.content
    );

    // SessionRouter populates these fields from the feishu webhook.
    let metadata = &result.metadata;
    assert!(metadata.contains_key("session_id"), "missing session_id");
    assert!(metadata.contains_key("from"), "missing from");
    assert!(metadata.contains_key("to"), "missing to");
    assert!(metadata.contains_key("channel"), "missing channel");
    assert!(metadata.contains_key("account_id"), "missing account_id");
}

// ---------------------------------------------------------------------------
// Test 2 — group chat rejected by SessionRouter
// ---------------------------------------------------------------------------

/// Verifies that group chat webhooks are rejected early by SessionRouter
/// (priority 20) with `ProcessError::SessionNotSupportedForChannel`.
#[tokio::test]
async fn test_group_chat_rejected() {
    let mgr = test_session_manager();
    let registry = ProcessorRegistry::new(mgr);
    let raw = load_raw_fixture("im-group-chat-message.json");

    let result = registry.process_inbound(&raw).await;

    let err = result.expect_err("group chat should be rejected");
    let err_msg = err.to_string();
    assert!(
        matches!(
            err,
            ProcessError::SessionNotSupportedForChannel(ref ch) if ch == "feishu"
        ),
        "expected SessionNotSupportedForChannel(\"feishu\"), got: {}",
        err_msg
    );
    assert!(
        err_msg.contains("feishu"),
        "error message should mention 'feishu': {}",
        err_msg
    );
}
