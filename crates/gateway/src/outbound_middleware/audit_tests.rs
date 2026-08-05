use super::audit::{build_payload_summary, type_name, AuditMiddleware};
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::middleware::{MiddlewareContext, OutboundMiddleware};

#[tokio::test]
async fn test_audit_allows_all_messages() {
    let mw = AuditMiddleware;
    let ctx = MiddlewareContext {
        session_id: "s1".into(),
        channel: "feishu".into(),
        chat_id: "c1".into(),
    };
    let rendered = RenderedOutput {
        msg_type: "text".into(),
        payload: serde_json::json!({"content": {"text": "hello"}}),
    };
    assert!(mw.process(&ctx, &rendered).await.is_ok());
}

#[tokio::test]
async fn test_payload_summary_interactive() {
    let rendered = RenderedOutput {
        msg_type: "interactive".into(),
        payload: serde_json::json!({
            "title": "alert",
            "blocks": [],
            "enabled": true
        }),
    };
    let summary = build_payload_summary(&rendered);
    assert!(summary.starts_with("interactive("));
    assert!(summary.contains("title:str"));
    assert!(summary.contains("blocks:arr"));
    assert!(summary.contains("enabled:bool"));
}

#[tokio::test]
async fn test_payload_summary_text_truncated() {
    let rendered = RenderedOutput {
        msg_type: "text".into(),
        payload: serde_json::json!({"content": {"text": "x".repeat(300)}}),
    };
    let summary = build_payload_summary(&rendered);
    // Truncated summary ends with "…" and is short
    assert!(
        summary.ends_with('…'),
        "expected truncation, got: {summary}"
    );
    assert!(
        summary.len() < 300,
        "expected short summary, got: {} chars",
        summary.len()
    );
}

#[test]
fn test_type_name() {
    assert_eq!(type_name(&serde_json::Value::Null), "null");
    assert_eq!(type_name(&serde_json::json!(true)), "bool");
    assert_eq!(type_name(&serde_json::json!(42)), "num");
    assert_eq!(type_name(&serde_json::json!("hi")), "str");
    assert_eq!(type_name(&serde_json::json!([])), "arr");
    assert_eq!(type_name(&serde_json::json!({})), "obj");
}
