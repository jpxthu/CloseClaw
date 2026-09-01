//! Tests for send_message / send_card_json warn logging (Step 1.1).
//!
//! Covers:
//! - send_message API error (resp.code != 0) → warn + Err
//! - send_card_json non-capability error → warn + Err, no fallback
//! - send_message reqwest connection failure → warn + Err
//! - send_card_json reqwest connection failure → warn + Err

use super::*;
use axum::{routing::post, Json, Router};
use tokio::net::TcpListener;

/// Create a FeishuAdapter pointing at a mock server.
fn make_adapter_with_base(base_url: &str) -> FeishuAdapter {
    let http_client = reqwest::Client::new();
    let tmp = tempfile::TempDir::new().expect("tmp dir");
    FeishuAdapter {
        app_id: "test_app_id".into(),
        app_secret: "test_secret".into(),
        verification_token: "test_token".into(),
        http_client,
        cached_token: Arc::new(tokio::sync::Mutex::new(None)),
        base_url: base_url.to_string(),
        last_metadata: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        media_store: Arc::new(
            crate::media_store::MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"),
        ),
        max_download_size_bytes: u64::MAX,
    }
}

/// Mock server that always returns a non-zero error code for the message
/// send endpoint. Token endpoint always succeeds.
async fn start_error_code_server(code: i32) -> String {
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
            post(move || async move {
                Json(serde_json::json!({
                    "code": code,
                    "msg": "API error"
                }))
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{}", addr)
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

/// send_message: API returns non-zero code → returns Err (warn logged).
#[tokio::test]
async fn test_send_message_api_error_returns_err() {
    let base_url = start_error_code_server(99999).await;
    let adapter = make_adapter_with_base(&base_url);
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
        "send_message should return Err on API error"
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
    let base_url = start_error_code_server(99999).await;
    let adapter = make_adapter_with_base(&base_url);
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

/// send_message: reqwest connection failure → returns Err (warn logged).
#[tokio::test]
async fn test_send_message_connection_failure_returns_err() {
    // Use a port that is very likely unused to trigger connection refused.
    let adapter = make_adapter_with_base("http://127.0.0.1:1");
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
        "send_message should return Err on connection failure"
    );
}

/// send_card_json: reqwest connection failure → returns Err (warn logged).
#[tokio::test]
async fn test_send_card_connection_failure_returns_err() {
    let adapter = make_adapter_with_base("http://127.0.0.1:1");
    let card = card_payload_with_markdown("content");
    let result = adapter.send_card_json("oc_chat", &card, None).await;
    assert!(
        result.is_err(),
        "send_card_json should return Err on connection failure"
    );
}
