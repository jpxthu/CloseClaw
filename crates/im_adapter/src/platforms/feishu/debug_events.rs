//! Debug events — trace_id generation and structured debug_log emission.
//!
//! Extracted from `plugin.rs` to keep impl blocks within the 100-line limit.

use chrono::Utc;
use closeclaw_debug_log::{LogEvent, LogLevel, TraceContext};
use tracing::warn;

use super::FeishuPlugin;

impl FeishuPlugin {
    /// Generate a trace_id in the format `{platform}_{timestamp_hex}_{uuid_v4}`.
    ///
    /// - Platform identifier: passed as `platform` parameter
    /// - Timestamp: Unix epoch milliseconds in hex
    /// - Random component: UUID v4 with hyphens removed
    ///
    /// This format allows operators to identify the source platform and approximate
    /// arrival time from the trace_id alone.
    pub(crate) fn generate_trace_id(&self, platform: &str) -> String {
        let timestamp_hex = format!("{:x}", Utc::now().timestamp_millis());
        let uuid_no_hyphens = uuid::Uuid::new_v4().simple().to_string();
        format!("{platform}_{timestamp_hex}_{uuid_no_hyphens}")
    }

    /// Emit a structured debug_log event asynchronously.
    ///
    /// Centralizes the repeated pattern: check debug_log, acquire trace_id,
    /// build event, spawn async send. Callers only supply `event_type` and
    /// `payload`. Skips silently when debug_log is None or trace_id is empty.
    pub(super) fn emit_debug_event(&self, event_type: &str, payload: serde_json::Value) {
        let debug_log = match self.debug_log {
            Some(ref dl) => dl.clone(),
            None => return,
        };
        let trace_id = self
            .adapter
            .last_metadata
            .try_lock()
            .ok()
            .and_then(|m| m.get("trace_id").cloned());
        match trace_id {
            Some(tid) if !tid.is_empty() => {
                let ctx = TraceContext::new_root(tid);
                let event =
                    LogEvent::new(&ctx, None, LogLevel::Info, "feishu", event_type, payload);
                tokio::spawn(async move {
                    debug_log.log(event).await;
                });
            }
            _ => {
                warn!(
                    event_type = %event_type,
                    "emit_debug_event: try_lock failed or trace_id empty — skipping"
                );
            }
        }
    }
}
