//! Unit tests for FeishuPlugin debug_log integration (Step 1.4).
//!
//! Covers:
//! 1. `set_debug_log` stores a DebugLog instance correctly
//! 2. `parse_inbound` works normally without DebugLog (no panic)
//! 3. `parse_inbound` works normally with DebugLog set (no panic)

use super::adapter::FeishuAdapter;
use super::adapter::FEISHU_API_BASE;
use super::FeishuPlugin;
use crate::IMPlugin;
use closeclaw_debug_log::{DebugLog, DebugLogConfig, LogLevel};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

/// Create a test FeishuAdapter (no real HTTP — only sync methods are exercised).
fn make_test_adapter() -> FeishuAdapter {
    let http_client = reqwest::Client::new();
    FeishuAdapter {
        app_id: "test_app_id".to_string(),
        app_secret: "test_secret".to_string(),
        verification_token: "test_token".to_string(),
        http_client,
        cached_token: Arc::new(tokio::sync::Mutex::new(None)),
        base_url: FEISHU_API_BASE.to_string(),
        last_metadata: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
    }
}

/// Create a DebugLog instance writing to a temp directory.
async fn make_debug_log(temp_dir: &TempDir) -> DebugLog {
    let config = DebugLogConfig {
        min_level: LogLevel::Trace,
        log_dir: temp_dir.path().to_path_buf(),
        retention_days: 1,
        redaction_patterns: vec![],
    };
    DebugLog::new(config).await.expect("DebugLog::new failed")
}

/// Build a webhook payload for a text message.
fn make_text_payload(text: &str) -> Vec<u8> {
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": "ev_debug_log_test",
            "event_type": "im.message.receive_v1",
            "create_time": "1234567890",
            "token": "tok",
            "app_id": "test_app_id"
        },
        "event": {
            "sender": {
                "sender_id": { "open_id": "ou_sender" },
                "sender_type": "user"
            },
            "content": serde_json::json!({"text": text}).to_string(),
            "chat_id": "oc_chat",
            "message_type": "text"
        }
    });
    serde_json::to_vec(&payload).unwrap()
}

// ===========================================================================
// set_debug_log tests
// ===========================================================================

/// set_debug_log correctly stores a DebugLog instance.
///
/// After calling set_debug_log, the plugin should hold the instance.
/// We verify by checking that parse_inbound still works correctly
/// (the DebugLog is an optional field; set_debug_log shouldn't break anything).
#[tokio::test]
async fn test_set_debug_log_stores_instance() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let debug_log = make_debug_log(&temp_dir).await;

    let adapter = Arc::new(make_test_adapter());
    let mut plugin = FeishuPlugin::new(adapter);

    // Before setting debug_log, parse_inbound should work fine.
    let payload = make_text_payload("before");
    let result = plugin.parse_inbound(&payload).await.unwrap();
    assert!(
        result.is_some(),
        "parse_inbound should return Some before set_debug_log"
    );

    // Set the debug_log.
    plugin.set_debug_log(Arc::new(debug_log));

    // After setting debug_log, parse_inbound should still work.
    let payload = make_text_payload("after");
    let result = plugin.parse_inbound(&payload).await.unwrap();
    assert!(
        result.is_some(),
        "parse_inbound should return Some after set_debug_log"
    );
}

/// set_debug_log can be called multiple times (last one wins).
#[tokio::test]
async fn test_set_debug_log_overwrite() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let debug_log_1 = make_debug_log(&temp_dir).await;
    let debug_log_2 = make_debug_log(&temp_dir).await;

    let adapter = Arc::new(make_test_adapter());
    let mut plugin = FeishuPlugin::new(adapter);

    plugin.set_debug_log(Arc::new(debug_log_1));
    plugin.set_debug_log(Arc::new(debug_log_2));

    // Should not panic — second set_debug_log replaces the first.
    let payload = make_text_payload("overwrite");
    let result = plugin.parse_inbound(&payload).await.unwrap();
    assert!(
        result.is_some(),
        "parse_inbound should work after set_debug_log overwrite"
    );
}

// ===========================================================================
// parse_inbound without DebugLog tests
// ===========================================================================

/// parse_inbound without DebugLog returns the parsed message normally (no panic).
#[tokio::test]
async fn test_parse_inbound_without_debug_log_no_panic() {
    let adapter = Arc::new(make_test_adapter());
    let plugin = FeishuPlugin::new(adapter);
    // debug_log is None by default.

    let payload = make_text_payload("hello without debug_log");
    let result = plugin.parse_inbound(&payload).await.unwrap();
    let msg = result.expect("parse_inbound should return Some for valid text payload");
    assert_eq!(msg.content, "hello without debug_log");
    assert_eq!(msg.platform, "feishu");
}

/// parse_inbound with empty text (discarded) without DebugLog returns None.
#[tokio::test]
async fn test_parse_inbound_empty_text_without_debug_log() {
    let adapter = Arc::new(make_test_adapter());
    let plugin = FeishuPlugin::new(adapter);

    let payload = make_text_payload("");
    let result = plugin.parse_inbound(&payload).await.unwrap();
    assert!(
        result.is_none(),
        "empty text should be discarded regardless of debug_log"
    );
}

/// parse_inbound with invalid payload without DebugLog returns error (no panic).
#[tokio::test]
async fn test_parse_inbound_invalid_payload_without_debug_log() {
    let adapter = Arc::new(make_test_adapter());
    let plugin = FeishuPlugin::new(adapter);

    let payload = b"not valid json";
    let result = plugin.parse_inbound(payload).await;
    assert!(result.is_err(), "invalid payload should return error");
}

// ===========================================================================
// parse_inbound with DebugLog tests
// ===========================================================================

/// parse_inbound with DebugLog returns the parsed message normally.
///
/// This verifies that having a DebugLog set doesn't interfere with parsing.
#[tokio::test]
async fn test_parse_inbound_with_debug_log_no_panic() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let debug_log = make_debug_log(&temp_dir).await;

    let adapter = Arc::new(make_test_adapter());
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let payload = make_text_payload("hello with debug_log");
    let result = plugin.parse_inbound(&payload).await.unwrap();
    let msg = result.expect("parse_inbound should return Some for valid text payload");
    assert_eq!(msg.content, "hello with debug_log");
}

/// parse_inbound with DebugLog set and empty text returns None (no panic).
#[tokio::test]
async fn test_parse_inbound_empty_text_with_debug_log() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let debug_log = make_debug_log(&temp_dir).await;

    let adapter = Arc::new(make_test_adapter());
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let payload = make_text_payload("");
    let result = plugin.parse_inbound(&payload).await.unwrap();
    assert!(
        result.is_none(),
        "empty text should be discarded even with DebugLog set"
    );
}

/// parse_inbound with DebugLog set and invalid payload returns error (no panic).
#[tokio::test]
async fn test_parse_inbound_invalid_payload_with_debug_log() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let debug_log = make_debug_log(&temp_dir).await;

    let adapter = Arc::new(make_test_adapter());
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let payload = b"not valid json";
    let result = plugin.parse_inbound(payload).await;
    assert!(
        result.is_err(),
        "invalid payload should return error regardless of debug_log"
    );
}
