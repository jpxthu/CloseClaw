//! Tests for IMPlugin::send fallback behaviour (Step 1.2).
//!
//! Covers:
//! - text send failure → warn + return Ok(())
//! - interactive send failure → text fallback → success
//! - interactive send failure → text fallback also fails → Ok(())
//! - unknown msg_type → Err(UnsupportedOperation)
//! - extract_card_plain_text correctness

use super::*;
use crate::plugin::IMPlugin;
use axum::{routing::post, Json, Router};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

/// Start a mock server that always returns success, return (base_url, JoinHandle).
async fn start_success_server() -> (String, tokio::task::JoinHandle<()>) {
    let app = Router::new()
        .route(
            "/auth/v3/tenant_access_token/internal",
            post(|| async {
                Json(serde_json::json!({
                    "code": 0,
                    "msg": "ok",
                    "tenant_access_token": "mock_token"
                }))
            }),
        )
        .route(
            "/im/v1/messages",
            post(|| async { Json(serde_json::json!({"code": 0, "msg": "ok"})) }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{}", addr), handle)
}

/// Start a mock server that always returns failure code 230001.
async fn start_fail_server() -> (String, tokio::task::JoinHandle<()>) {
    let app = Router::new()
        .route(
            "/auth/v3/tenant_access_token/internal",
            post(|| async {
                Json(serde_json::json!({
                    "code": 0,
                    "msg": "ok",
                    "tenant_access_token": "mock_token"
                }))
            }),
        )
        .route(
            "/im/v1/messages",
            post(|| async {
                Json(serde_json::json!({
                    "code": 230001,
                    "msg": "permission denied"
                }))
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{}", addr), handle)
}

/// Start a mock server that fails first `fail_count` calls then succeeds.
async fn start_fail_then_succeed_server(
    fail_count: usize,
) -> (String, tokio::task::JoinHandle<()>) {
    let call_count = Arc::new(Mutex::new(0usize));
    let fail_count = Arc::new(fail_count);
    let app = Router::new()
        .route(
            "/auth/v3/tenant_access_token/internal",
            post(|| async {
                Json(serde_json::json!({
                    "code": 0,
                    "msg": "ok",
                    "tenant_access_token": "mock_token"
                }))
            }),
        )
        .route(
            "/im/v1/messages",
            {
                let cc = call_count.clone();
                let fc = fail_count.clone();
                post(move || async move {
                    let mut count = cc.lock().await;
                    *count += 1;
                    if *count <= *fc {
                        Json(serde_json::json!({
                            "code": 230001,
                            "msg": "permission denied"
                        }))
                    } else {
                        Json(serde_json::json!({"code": 0, "msg": "ok"}))
                    }
                })
            },
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{}", addr), handle)
}

fn make_plugin(base_url: &str) -> FeishuPlugin {
    let adapter = Arc::new(FeishuAdapter {
        app_id: "test".into(),
        app_secret: "test".into(),
        verification_token: "test".into(),
        http_client: reqwest::Client::new(),
        cached_token: Arc::new(tokio::sync::Mutex::new(None)),
        base_url: base_url.to_string(),
        last_metadata: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
    });
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
// extract_card_plain_text tests (sync, no mock server needed)
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
// IMPlugin::send fallback tests (async, use mock servers)
// =====================================================================

#[tokio::test]
async fn text_send_failure_returns_ok() {
    let (url, _server) = start_fail_server().await;
    let plugin = make_plugin(&url);
    let output = text_output("hello");
    let result = plugin.send(&output, "chat_test", None).await;
    assert!(result.is_ok(), "text send failure should return Ok(())");
}

#[tokio::test]
async fn interactive_send_failure_falls_back_to_text_success() {
    // First call (interactive) fails, second call (text fallback) succeeds
    let (url, _server) = start_fail_then_succeed_server(1).await;
    let plugin = make_plugin(&url);
    let output = interactive_output("fallback content");
    let result = plugin.send(&output, "chat_test", None).await;
    assert!(
        result.is_ok(),
        "interactive failure with text fallback success should return Ok(())"
    );
}

#[tokio::test]
async fn interactive_send_failure_text_fallback_also_fails_returns_ok() {
    let (url, _server) = start_fail_server().await;
    let plugin = make_plugin(&url);
    let output = interactive_output("content to extract");
    let result = plugin.send(&output, "chat_test", None).await;
    assert!(
        result.is_ok(),
        "both interactive and text fallback failing should return Ok(())"
    );
}

#[tokio::test]
async fn unknown_msg_type_returns_err() {
    let (url, _server) = start_success_server().await;
    let plugin = make_plugin(&url);
    let output = RenderedOutput {
        msg_type: "unknown_type".into(),
        payload: serde_json::json!({}),
    };
    let result = plugin.send(&output, "chat_test", None).await;
    assert!(result.is_err(), "unknown msg_type should return Err");
    match result.unwrap_err() {
        CommonAdapterError::UnsupportedOperation => {}
        other => panic!("expected UnsupportedOperation, got {:?}", other),
    }
}

#[tokio::test]
async fn interactive_empty_text_fallback_returns_ok() {
    // Card with no extractable text → empty plain_text → return Ok(())
    let (url, _server) = start_fail_server().await;
    let plugin = make_plugin(&url);
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
    let result = plugin.send(&output, "chat_test", None).await;
    assert!(result.is_ok(), "empty text fallback should return Ok(())");
}
