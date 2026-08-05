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
pub(crate) fn build_payload_summary(rendered: &RenderedOutput) -> String {
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
pub(crate) fn type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "num",
        serde_json::Value::String(_) => "str",
        serde_json::Value::Array(_) => "arr",
        serde_json::Value::Object(_) => "obj",
    }
}
