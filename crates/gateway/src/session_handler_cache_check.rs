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
use closeclaw_llm::cache_adapter::provider_cache_ttl_ms;
use closeclaw_session::llm_session::ConversationSession;

use crate::outbound::StreamResult;

/// Current Unix epoch time in milliseconds (same clock as
/// `RunningStats::accumulate`'s `last_call_at`).
fn current_epoch_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Runs the round's cache hit consistency check, reports any anomaly,
/// and accumulates the round's usage.
///
/// Extraction of the former `clear_busy_and_send` inline block; the
/// anomaly-disposal behavior is unchanged: warn log →
/// [`MetricsEmitter::emit_cache_anomaly`] → Owner system notification
/// ([`NotificationPriority::Now`]).
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
        current_epoch_millis(),
        ttl_ms,
    ) {
        tracing::warn!(
            session_id,
            previous_prompt_tokens = info.previous_prompt_tokens,
            current_cache_read_tokens = info.current_cache_read_tokens,
            deviation = info.deviation_tokens,
            tolerance = info.tolerance_tokens,
            "Cache anomaly detected"
        );
        if let Some(emitter) = metrics_emitter {
            emitter.emit_cache_anomaly(&info);
        }
        let notification = info.format_notification();
        cs.push_system_notification(notification, NotificationPriority::Now);
    }
    cs.accumulate_usage(&stream_result.usage, Some(&call_model));
}
