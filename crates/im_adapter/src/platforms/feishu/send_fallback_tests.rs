//! Tests for IMPlugin::send fallback behaviour (Step 1.2).
//!
//! Covers:
//! - text send failure → warn + return Ok(())
//! - interactive send failure → text fallback → success
//! - interactive send failure → text fallback also fails → Ok(())
//! - unknown msg_type → Err(UnsupportedOperation)
//! - extract_card_plain_text correctness
//! - send_card_json capability error fallback

use super::*;
use crate::plugin::IMPlugin;
use std::io::Write;
use std::sync::Arc;
use tempfile::TempDir;

/// Create a mock lark-cli script that outputs the given JSON to stdout.
fn create_mock_cli(tmp: &TempDir, response_json: &str) -> String {
    let script_path = tmp.path().join("mock_cli.sh");
    let mut f = std::fs::File::create(&script_path).unwrap();
    writeln!(f, "#!/bin/bash").unwrap();
    writeln!(f, "echo '{response_json}'").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, PermissionsExt::from_mode(0o755)).unwrap();
    }
    script_path.to_str().unwrap().to_string()
}

/// Create a mock lark-cli script that always exits with error.
fn create_failing_mock_cli(tmp: &TempDir) -> String {
    let script_path = tmp.path().join("failing_cli.sh");
    let mut f = std::fs::File::create(&script_path).unwrap();
    writeln!(f, "#!/bin/bash").unwrap();
    writeln!(
        f,
        "echo '{{\"code\":230001,\"msg\":\"capability error\"}}' >&2"
    )
    .unwrap();
    writeln!(f, "echo '{{\"code\":230001,\"msg\":\"capability error\"}}'").unwrap();
    writeln!(f, "exit 1").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, PermissionsExt::from_mode(0o755)).unwrap();
    }
    script_path.to_str().unwrap().to_string()
}

/// Create a mock lark-cli script that fails for the first `fail_count` calls,
/// then succeeds. Uses a counter file to track calls.
fn create_fail_then_succeed_cli(tmp: &TempDir, fail_count: usize) -> String {
    let script_path = tmp.path().join("fail_then_succeed.sh");
    let counter_path = tmp.path().join("call_counter");
    let fail_count_str = fail_count.to_string();
    let mut f = std::fs::File::create(&script_path).unwrap();
    writeln!(f, "#!/bin/bash").unwrap();
    writeln!(f, "COUNTER_FILE=\"{}\"", counter_path.display()).unwrap();
    writeln!(f, "COUNT=$(cat \"$COUNTER_FILE\" 2>/dev/null || echo 0)").unwrap();
    writeln!(f, "echo $((COUNT + 1)) > \"$COUNTER_FILE\"").unwrap();
    writeln!(f, "if [ \"$COUNT\" -lt {} ]; then", fail_count_str).unwrap();
    writeln!(
        f,
        "  echo '{{\"code\":230002,\"msg\":\"capability error\"}}' >&2"
    )
    .unwrap();
    writeln!(
        f,
        "  echo '{{\"code\":230002,\"msg\":\"capability error\"}}'"
    )
    .unwrap();
    writeln!(f, "  exit 1").unwrap();
    writeln!(f, "else").unwrap();
    writeln!(f, "  echo '{{\"code\":0,\"msg\":\"ok\"}}'").unwrap();
    writeln!(f, "fi").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, PermissionsExt::from_mode(0o755)).unwrap();
    }
    // Initialize counter
    std::fs::write(&counter_path, "0").unwrap();
    script_path.to_str().unwrap().to_string()
}

/// Create a FeishuPlugin with a mock lark-cli.
fn make_plugin(cli_command: &str) -> FeishuPlugin {
    let tmp = tempfile::TempDir::new().expect("tmp dir");
    let adapter = Arc::new(FeishuAdapter::new(
        "test_profile".into(),
        Arc::new(
            crate::media_store::MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"),
        ),
    ));
    // Override cli_command for mock testing
    let adapter = {
        let mut a = (*adapter).clone();
        a.cli_command = cli_command.to_string();
        Arc::new(a)
    };
    FeishuPlugin::new(adapter)
}

fn text_output(content: &str) -> RenderedOutput {
    RenderedOutput {
        msg_type: "text".into(),
        payload: serde_json::json!({
            "msg_type": "text",
            "content": { "text": content }
        }),
    }
}

fn interactive_output(markdown_content: &str) -> RenderedOutput {
    RenderedOutput {
        msg_type: "interactive".into(),
        payload: serde_json::json!({
            "msg_type": "interactive",
            "card": {
                "elements": [
                    { "tag": "markdown", "content": markdown_content }
                ]
            }
        }),
    }
}

// =====================================================================
// extract_card_plain_text tests (sync, no mock CLI needed)
// =====================================================================

#[test]
fn extract_card_plain_text_extracts_markdown() {
    let payload = serde_json::json!({
        "card": {
            "elements": [
                { "tag": "markdown", "content": "Hello world" }
            ]
        }
    });
    assert_eq!(extract_card_plain_text(&payload), "Hello world");
}

#[test]
fn extract_card_plain_text_extracts_plain_text() {
    let payload = serde_json::json!({
        "card": {
            "elements": [
                { "tag": "plain_text", "content": "Simple text" }
            ]
        }
    });
    assert_eq!(extract_card_plain_text(&payload), "Simple text");
}

#[test]
fn extract_card_plain_text_joins_multiple_elements() {
    let payload = serde_json::json!({
        "card": {
            "elements": [
                { "tag": "markdown", "content": "Line 1" },
                { "tag": "plain_text", "content": "Line 2" },
                { "tag": "hr" }
            ]
        }
    });
    assert_eq!(extract_card_plain_text(&payload), "Line 1\nLine 2");
}

#[test]
fn extract_card_plain_text_extracts_button_text() {
    let payload = serde_json::json!({
        "card": {
            "elements": [
                {
                    "tag": "action",
                    "actions": [
                        { "tag": "button", "text": { "tag": "plain_text", "content": "Click me" } }
                    ]
                }
            ]
        }
    });
    assert_eq!(extract_card_plain_text(&payload), "Click me");
}

#[test]
fn extract_card_plain_text_empty_payload() {
    let payload = serde_json::json!({});
    assert_eq!(extract_card_plain_text(&payload), "");
}

#[test]
fn extract_card_plain_text_no_elements() {
    let payload = serde_json::json!({"card": {}});
    assert_eq!(extract_card_plain_text(&payload), "");
}

// =====================================================================
// IMPlugin::send fallback tests (async, use mock CLI scripts)
// =====================================================================

/// text send: mock CLI fails → warn + return Ok(()) per design doc.
#[tokio::test]
async fn text_send_failure_returns_ok() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp);
    let plugin = make_plugin(&cli);
    let output = text_output("hello");
    let result = plugin.send(&output, "chat_test", None, None).await;
    assert!(result.is_ok(), "text send failure should return Ok(())");
}

/// interactive send: mock CLI fails with capability error → text fallback succeeds.
#[tokio::test]
async fn interactive_send_failure_falls_back_to_text_success() {
    let tmp = TempDir::new().unwrap();
    // First call (interactive) fails with capability error, second call (text) succeeds
    let cli = create_fail_then_succeed_cli(&tmp, 1);
    let plugin = make_plugin(&cli);
    let output = interactive_output("fallback content");
    let result = plugin.send(&output, "chat_test", None, None).await;
    assert!(
        result.is_ok(),
        "interactive failure with text fallback success should return Ok(())"
    );
}

/// interactive send: mock CLI fails → text fallback also fails → Ok(())
#[tokio::test]
async fn interactive_send_failure_text_fallback_also_fails_returns_ok() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp);
    let plugin = make_plugin(&cli);
    let output = interactive_output("content to extract");
    let result = plugin.send(&output, "chat_test", None, None).await;
    assert!(
        result.is_ok(),
        "both interactive and text fallback failing should return Ok(())"
    );
}

/// unknown msg_type → Err(UnsupportedOperation)
#[tokio::test]
async fn unknown_msg_type_returns_err() {
    let tmp = TempDir::new().unwrap();
    let cli = create_mock_cli(&tmp, r#"{"code":0,"msg":"ok"}"#);
    let plugin = make_plugin(&cli);
    let output = RenderedOutput {
        msg_type: "unknown_type".into(),
        payload: serde_json::json!({}),
    };
    let result = plugin.send(&output, "chat_test", None, None).await;
    assert!(result.is_err(), "unknown msg_type should return Err");
    match result.unwrap_err() {
        CommonAdapterError::UnsupportedOperation => {}
        other => panic!("expected UnsupportedOperation, got {:?}", other),
    }
}

/// Card with no extractable text → empty plain_text → return Ok(())
#[tokio::test]
async fn interactive_empty_text_fallback_returns_ok() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp);
    let plugin = make_plugin(&cli);
    let output = RenderedOutput {
        msg_type: "interactive".into(),
        payload: serde_json::json!({
            "msg_type": "interactive",
            "card": {
                "elements": [
                    { "tag": "hr" }
                ]
            }
        }),
    };
    let result = plugin.send(&output, "chat_test", None, None).await;
    assert!(result.is_ok(), "empty text fallback should return Ok(())");
}

// =====================================================================
// send_card_json capability fallback tests (via adapter)
// =====================================================================

/// Helper: create a FeishuAdapter pointing at a mock CLI.
fn make_adapter(cli_command: &str) -> FeishuAdapter {
    let tmp = tempfile::TempDir::new().unwrap();
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

#[tokio::test]
async fn send_card_capability_error_fallback_to_text_succeeds() {
    let tmp = TempDir::new().unwrap();
    let cli = create_fail_then_succeed_cli(&tmp, 1);
    let adapter = make_adapter(&cli);
    let card = card_payload_with_markdown("fallback content");
    let result = adapter.send_card_json("oc_chat", &card, None).await;
    assert!(
        result.is_ok(),
        "should return Ok when card fails but text fallback succeeds — got {:?}",
        result
    );
}

#[tokio::test]
async fn send_card_capability_error_fallback_text_also_fails() {
    let tmp = TempDir::new().unwrap();
    // Both card and text fallback fail with capability error
    let cli = create_failing_mock_cli(&tmp);
    let adapter = make_adapter(&cli);
    let card = card_payload_with_markdown("content");
    let result = adapter.send_card_json("oc_chat", &card, None).await;
    assert!(
        result.is_ok(),
        "should return Ok when both card and text fail (fallback returns Ok)"
    );
}

#[tokio::test]
async fn send_card_non_capability_error_returns_err() {
    let tmp = TempDir::new().unwrap();
    let cli = create_mock_cli(&tmp, r#"{"code":99999,"msg":"unknown error"}"#);
    let adapter = make_adapter(&cli);
    let card = card_payload_with_markdown("content");
    let result = adapter.send_card_json("oc_chat", &card, None).await;
    assert!(
        result.is_err(),
        "should return Err for non-capability error"
    );
}

#[tokio::test]
async fn send_card_success_no_fallback() {
    let tmp = TempDir::new().unwrap();
    let cli = create_mock_cli(&tmp, r#"{"code":0,"msg":"ok"}"#);
    let adapter = make_adapter(&cli);
    let card = card_payload_with_markdown("content");
    let result = adapter.send_card_json("oc_chat", &card, None).await;
    assert!(result.is_ok(), "successful card send should return Ok");
}
