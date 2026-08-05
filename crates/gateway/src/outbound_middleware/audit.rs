//! Audit middleware — records every outbound message for audit purposes.
//!
//! Logs session ID, channel, message type, and a payload summary at
//! `tracing::info!` level. Interactive messages get an additional
//! summary of their payload keys.

use async_trait::async_trait;
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::middleware::{MiddlewareContext, MiddlewareError, OutboundMiddleware};

/// Outbound audit middleware that logs every message before send.
///
/// The audit trail is recorded at `tracing::info!` level so it can be
/// captured by the configured log subscriber (e.g. structured JSON log
/// file). Interactive payloads produce a summary of top-level keys for
/// quick inspection without dumping the full JSON body.
pub struct AuditMiddleware;

#[async_trait]
impl OutboundMiddleware for AuditMiddleware {
    fn name(&self) -> &str {
        "audit"
    }

    async fn process(
        &self,
        ctx: &MiddlewareContext,
        rendered: &RenderedOutput,
    ) -> Result<(), MiddlewareError> {
        let summary = build_payload_summary(rendered);

        tracing::info!(
            session_id = %ctx.session_id,
            channel = %ctx.channel,
            chat_id = %ctx.chat_id,
            msg_type = %rendered.msg_type,
            payload_summary = %summary,
            "outbound audit"
        );

        Ok(())
    }
}

/// Build a short human-readable summary of the rendered payload.
///
/// For `interactive` messages the summary lists the top-level keys and
/// their types. For other message types the full payload is truncated to
/// a reasonable length.
fn build_payload_summary(rendered: &RenderedOutput) -> String {
    match rendered.msg_type.as_str() {
        "interactive" => {
            let keys: Vec<String> = rendered
                .payload
                .as_object()
                .map(|m| {
                    m.keys()
                        .map(|k| format!("{}:{}", k, type_name(&m[k])))
                        .collect()
                })
                .unwrap_or_default();
            format!("interactive({})", keys.join(","))
        }
        _ => {
            let s = rendered.payload.to_string();
            if s.len() > 200 {
                format!("{}…", &s[..200])
            } else {
                s
            }
        }
    }
}

/// Return a short type label for a JSON value.
fn type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "num",
        serde_json::Value::String(_) => "str",
        serde_json::Value::Array(_) => "arr",
        serde_json::Value::Object(_) => "obj",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_common::im_plugin::RenderedOutput;

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
}
