//! Tests for send_message / send_card_json warn logging (Step 1.2).
//!
//! Covers:
//! - send_message lark-cli error → warn + Err
//! - send_card_json non-capability error → warn + Err, no fallback
//! - send_message lark-cli command not found → warn + Err
//! - send_card_json lark-cli command not found → warn + Err
//!
//! Convention (see `test_helpers.rs` module docs): any test that creates and
//! actually spawns a mock CLI script must be marked `#[serial_test::serial]`
//! (ETXTBSY root cause); mock scripts must live in a `tempfile::TempDir`.

use super::*;
use crate::error::AdapterError;
use crate::IMAdapter;
use serial_test::serial;
use tempfile::TempDir;

/// Helper: build a card JSON string with a single markdown element.
fn card_payload_with_markdown(text: &str) -> String {
    serde_json::json!({
        "msg_type": "interactive",
        "card": {
            "elements": [
                { "tag": "markdown", "content": text }
            ]
        }
    })
    .to_string()
}

/// send_message: mock CLI returns error code → returns Err (warn logged).
#[serial]
#[tokio::test]
async fn test_send_message_cli_error_returns_err() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(
        &tmp,
        "error_cli.sh",
        r#"{"code":99999,"msg":"API error"}"#,
        true,
        1,
    );
    let adapter = make_adapter(&cli);
    let result = adapter.send_message("oc_target_chat", "hello", None).await;
    assert!(
        result.is_err(),
        "send_message should return Err on CLI error"
    );
    match result.unwrap_err() {
        AdapterError::SendFailed(msg) => {
            assert!(msg.contains("99999"), "error should contain API code");
        }
        other => panic!("expected SendFailed, got {:?}", other),
    }
}

/// send_card_json: non-capability error (code 99999) → warn + return Err,
/// no text fallback attempted.
#[serial]
#[tokio::test]
async fn test_send_card_non_capability_error_returns_err_no_fallback() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(
        &tmp,
        "error_cli.sh",
        r#"{"code":99999,"msg":"API error"}"#,
        true,
        1,
    );
    let adapter = make_adapter(&cli);
    let card = card_payload_with_markdown("content");
    let result = adapter.send_card_json("oc_chat", &card, None).await;
    assert!(
        result.is_err(),
        "send_card_json should return Err on non-capability error"
    );
    match result.unwrap_err() {
        AdapterError::SendFailed(msg) => {
            assert!(msg.contains("99999"), "error should contain API code");
        }
        other => panic!("expected SendFailed, got {:?}", other),
    }
}

/// send_message: lark-cli command not found → returns Err (warn logged).
#[tokio::test]
async fn test_send_message_command_not_found_returns_err() {
    let adapter = make_adapter("nonexistent_command_xyz");
    let result = adapter.send_message("oc_target_chat", "hello", None).await;
    assert!(
        result.is_err(),
        "send_message should return Err when command not found"
    );
}

/// send_card_json: lark-cli command not found → returns Err (warn logged).
#[tokio::test]
async fn test_send_card_command_not_found_returns_err() {
    let adapter = make_adapter("nonexistent_command_xyz");
    let card = card_payload_with_markdown("content");
    let result = adapter.send_card_json("oc_chat", &card, None).await;
    assert!(
        result.is_err(),
        "send_card_json should return Err when command not found"
    );
}
