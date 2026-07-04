//! IM inbound chain E2E tests — ProcessorRegistry with real fixtures.
//!
//! Covers:
//! - p2p DM: raw webhook → ProcessorRegistry → processed output
//!
//! NOTE: These tests were adapted from the old `im::processor` module
//! (Step 1.4). The old group-chat rejection test was removed because
//! the new `processor_chain` SessionRouter does not reject group chats
//! (design doc requirement: "SessionRouter 不区分私聊和群聊").
//!
//! Step 1.5 removed `RawMessage`; this file now uses `NormalizedMessage`.
//!
//! Run with: `cargo test --test im_inbound_e2e_tests`

use std::path::PathBuf;

use closeclaw::processor_chain::NormalizedMessage;
use closeclaw::processor_chain::ProcessorRegistry;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn feishu_fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/feishu")
}

fn load_raw_fixture(filename: &str) -> serde_json::Value {
    let path = feishu_fixtures_dir().join(filename);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

fn webhook_to_normalized_message(webhook: &serde_json::Value) -> NormalizedMessage {
    let content = webhook
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let sender_id = webhook
        .get("sender")
        .and_then(|s| s.get("sender_id"))
        .and_then(|sid| sid.get("open_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let peer_id = webhook
        .get("message")
        .and_then(|m| m.get("chat_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let message_id = webhook
        .get("message")
        .and_then(|m| m.get("message_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id,
        peer_id,
        content,
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        account_id: message_id,
    }
}

// ---------------------------------------------------------------------------
// Test 1 — p2p DM full chain
// ---------------------------------------------------------------------------

/// Verifies the p2p DM inbound chain processes successfully.
/// The empty registry bypasses processors and returns the raw content.
#[tokio::test]
async fn test_p2p_dm_full_chain() {
    let registry = ProcessorRegistry::new();
    let raw = load_raw_fixture("im-message-receive_v1-no-event-id-2026-04-26T18-53-09-967Z.json");
    let msg = webhook_to_normalized_message(&raw);

    let result = registry
        .process_inbound(msg)
        .await
        .expect("p2p chain should succeed");

    // Empty registry returns raw content as-is (bypass mode).
    let text = result.text_content().unwrap_or("");
    assert!(!text.is_empty(), "text content should not be empty");
}
