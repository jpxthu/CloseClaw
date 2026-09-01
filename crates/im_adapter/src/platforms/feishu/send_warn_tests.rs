//! Tests for send_message / send_card_json warn logging (Step 1.2).
//!
//! Covers:
//! - send_message lark-cli error → warn + Err
//! - send_card_json non-capability error → warn + Err, no fallback
//! - send_message lark-cli command not found → warn + Err
//! - send_card_json lark-cli command not found → warn + Err

use super::*;
use std::io::Write;
use tempfile::TempDir;

/// Create a mock lark-cli script that outputs an error JSON.
fn create_error_mock_cli(tmp: &TempDir, code: i32) -> String {
    let script_path = tmp.path().join("error_cli.sh");
    let mut f = std::fs::File::create(&script_path).unwrap();
    writeln!(f, "#!/bin/bash").unwrap();
    writeln!(f, "echo '{{\"code\":{code},\"msg\":\"API error\"}}' >&2").unwrap();
    writeln!(f, "echo '{{\"code\":{code},\"msg\":\"API error\"}}'").unwrap();
    writeln!(f, "exit 1").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, PermissionsExt::from_mode(0o755)).unwrap();
    }
    script_path.to_str().unwrap().to_string()
}

/// Create a FeishuAdapter pointing at a mock CLI script.
fn make_adapter_with_cli(cli_command: &str) -> FeishuAdapter {
    let tmp = tempfile::TempDir::new().expect("tmp dir");
    let mut adapter = FeishuAdapter::new(
        "test_profile".into(),
        Arc::new(
            crate::media_store::MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"),
        ),
    );
    adapter.cli_command = cli_command.to_string();
    adapter
}

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
#[tokio::test]
async fn test_send_message_cli_error_returns_err() {
    let tmp = TempDir::new().unwrap();
    let cli = create_error_mock_cli(&tmp, 99999);
    let adapter = make_adapter_with_cli(&cli);
    let msg = Message {
        id: "1".into(),
        from: "a".into(),
        to: "oc_target_chat".into(),
        content: "hello".into(),
        channel: "feishu".into(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let result = adapter.send_message(&msg, None).await;
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
#[tokio::test]
async fn test_send_card_non_capability_error_returns_err_no_fallback() {
    let tmp = TempDir::new().unwrap();
    let cli = create_error_mock_cli(&tmp, 99999);
    let adapter = make_adapter_with_cli(&cli);
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
    let adapter = make_adapter_with_cli("nonexistent_command_xyz");
    let msg = Message {
        id: "1".into(),
        from: "a".into(),
        to: "oc_target_chat".into(),
        content: "hello".into(),
        channel: "feishu".into(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let result = adapter.send_message(&msg, None).await;
    assert!(
        result.is_err(),
        "send_message should return Err when command not found"
    );
}

/// send_card_json: lark-cli command not found → returns Err (warn logged).
#[tokio::test]
async fn test_send_card_command_not_found_returns_err() {
    let adapter = make_adapter_with_cli("nonexistent_command_xyz");
    let card = card_payload_with_markdown("content");
    let result = adapter.send_card_json("oc_chat", &card, None).await;
    assert!(
        result.is_err(),
        "send_card_json should return Err when command not found"
    );
}
