//! Gateway-side cache hit consistency check at the unified advancement
//! point.
//!
//! Extracted from `session_handler_announce.rs::clear_busy_and_send`
//! (the only place where the streaming and non-streaming paths
//! converge). Runs once the round's usage has fully arrived and must
//! run before usage accumulation, so the session-side judgment reads
//! the previous call's reference state
//! ([llm-session-enhancements](../../../docs/design/session/llm-session-enhancements.md)
//! "缓存命中一致性校验", doc :73).

use std::sync::Arc;

use closeclaw_common::MetricsEmitter;
use closeclaw_common::NotificationPriority;
use closeclaw_debug_log::DebugLog;
use closeclaw_debug_log::LogLevel;
use closeclaw_llm::cache_adapter::provider_cache_ttl_ms;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::llm_session::NotificationAudience;

use crate::debug_log_emitter::{emit_debug_event, DebugLogContext, EmitEventParams};
use crate::outbound::StreamResult;

/// Runs the round's cache hit consistency check, reports any anomaly,
/// and accumulates the round's usage.
///
/// Extraction of the former `clear_busy_and_send` inline block; the
/// anomaly-disposal behavior: error log → structured debug_log event
/// (`session.cache_anomaly.detected`, level Error — debug_log README
/// 「异常」级语义，缓存命中一致性校验判定为缓存异常) →
/// [`MetricsEmitter::emit_cache_anomaly`] → Owner-directed system
/// notification — declared [`NotificationAudience::OwnerOnly`] +
/// [`NotificationPriority::Now`] (outbound-flow「定向收件人」:
/// session 侧只声明，Gateway 在投递时依据当前 User 角色判定；
/// [llm-session-enhancements](../../../docs/design/session/llm-session-enhancements.md)
/// :87「当且仅当当前 User 为 Owner 时……发送系统通知」).
///
/// The debug_log event correlates by `session_id` (trace_id and
/// session_key both carry it): neither call site holds the inbound
/// message's trace metadata, so the session is the stable correlation
/// key, matching the `session.compaction.*` / `session.child.created`
/// family's `source_module="session"` attribution.
///
/// - The check ([`ConversationSession::check_cache_anomaly_and_advance`])
///   must run before `accumulate_usage` so `last_prompt_tokens` /
///   `last_call_model` / `last_call_at` still hold the previous call's
///   reference state (session-side contract).
/// - The model/provider identifier is the actually-used call target
///   reported by the invocation, degrading to the session-config model
///   when the caller reports no target (test fakes / stub paths); the
///   same identifier is passed to `accumulate_usage`.
/// - The provider cache TTL is resolved by
///   [`provider_cache_ttl_ms`] from the actually-used provider id;
///   unrecorded providers use the generic 5-minute default (doc :82).
pub(crate) fn check_cache_anomaly_and_accumulate_usage(
    session_id: &str,
    cs: &mut ConversationSession,
    stream_result: &StreamResult,
    metrics_emitter: &Option<Arc<dyn MetricsEmitter>>,
    debug_log: Option<&DebugLog>,
) {
    let call_model = stream_result
        .call_target
        .as_ref()
        .map(|t| t.id_string())
        .unwrap_or_else(|| cs.model().to_string());
    let ttl_ms = provider_cache_ttl_ms(
        stream_result
            .call_target
            .as_ref()
            .map(|t| t.provider_id.as_str())
            .unwrap_or_default(),
    );
    if let Some(info) = cs.check_cache_anomaly_and_advance(
        &stream_result.usage,
        Some(&call_model),
        closeclaw_common::current_epoch_millis(),
        ttl_ms,
    ) {
        tracing::error!(
            session_id,
            previous_prompt_tokens = info.previous_prompt_tokens,
            current_cache_read_tokens = info.current_cache_read_tokens,
            deviation = info.deviation_tokens,
            tolerance = info.tolerance_tokens,
            "Cache anomaly detected"
        );
        // Structured debug_log event (debug_log README :24,32): cache
        // anomaly is an「异常」(Error-level) event. No-op when the
        // caller has no debug_log instance (aligned with
        // `closeclaw_debug_log::emit_event` semantics).
        emit_debug_event(EmitEventParams {
            ctx: DebugLogContext::new(debug_log, session_id, Some(session_id)),
            level: LogLevel::Error,
            source_module: "session",
            event_type: "session.cache_anomaly.detected",
            payload: serde_json::json!({
                "previous_prompt_tokens": info.previous_prompt_tokens,
                "current_cache_read_tokens": info.current_cache_read_tokens,
                "deviation_tokens": info.deviation_tokens,
                "tolerance_tokens": info.tolerance_tokens,
            }),
            parent: None,
        });
        if let Some(emitter) = metrics_emitter {
            emitter.emit_cache_anomaly(&info);
        }
        let notification = info.format_notification();
        // Owner-directed audience declaration (outbound-flow「定向收件人」):
        // this disposal only declares; the Gateway performs the recipient
        // judgment against the session's current user at delivery time.
        cs.push_system_notification_with_audience(
            notification,
            NotificationPriority::Now,
            NotificationAudience::OwnerOnly,
        );
    }
    cs.accumulate_usage(&stream_result.usage, Some(&call_model));
}
