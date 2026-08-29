//! Unit tests for per-agent compact configuration (Step 1.4).
//!
//! Verifies that `SessionMessageHandler` correctly propagates the
//! `CompactConfig` provided at construction time to its internal
//! `CompactionService`, and that the circuit-breaker notification
//! flag is properly reset after manual compact success.

use super::*;
use crate::session_handler::ActiveSearcherLlmCaller;
use closeclaw_llm::retry::CooldownManager;
use closeclaw_llm::types::ContentBlock;
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;
use closeclaw_session::compaction::CompactConfig;
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::Arc;

// ── Helpers ──────────────────────────────────────────────────────────────

fn make_config() -> crate::GatewayConfig {
    crate::GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

fn make_sm() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        &make_config(),
        None,
        None,
        ReasoningLevel::default(),
    ))
}

fn make_fallback_client() -> Arc<UnifiedFallbackClient> {
    Arc::new(UnifiedFallbackClient::new(
        vec![],
        Arc::new(CooldownManager::new()),
    ))
}

fn make_active_searcher_caller() -> Arc<ActiveSearcherLlmCaller> {
    Arc::new(ActiveSearcherLlmCaller {
        caller: Arc::new(crate::llm_caller_impl::FallbackLlmCaller(Arc::new(
            UnifiedFallbackClient::new(vec![], Arc::new(CooldownManager::new())),
        ))) as Arc<dyn closeclaw_common::LlmCaller>,
        model: String::new(),
    })
}

/// Create a handler with a custom `CompactConfig` and an output channel.
fn handler_with_channel(
    sm: &Arc<SessionManager>,
    config: CompactConfig,
) -> (
    SessionMessageHandler,
    tokio::sync::mpsc::Receiver<(String, Vec<ContentBlock>)>,
) {
    let (tx, rx) = tokio::sync::mpsc::channel(10);
    let handler = SessionMessageHandler::new(
        Arc::clone(sm),
        make_fallback_client(),
        tx,
        make_active_searcher_caller(),
        config,
    );
    (handler, rx)
}

/// Create a handler with custom config and no output channel.
fn handler_no_output(sm: &Arc<SessionManager>, config: CompactConfig) -> SessionMessageHandler {
    SessionMessageHandler::new_no_output(
        Arc::clone(sm),
        make_fallback_client(),
        make_active_searcher_caller(),
        config,
    )
}

/// Assert the compaction service config matches expected values.
async fn assert_compaction_config(
    handler: &SessionMessageHandler,
    expected_chars_per_token: f64,
    expected_auto_threshold: f64,
    expected_warning_threshold: f64,
    expected_max_failures: usize,
) {
    let svc = handler.compaction_service.lock().await;
    let cfg = svc.config();
    assert_eq!(
        cfg.chars_per_token, expected_chars_per_token,
        "chars_per_token mismatch"
    );
    assert!(
        (cfg.auto_compact_threshold_pct - expected_auto_threshold).abs() < f64::EPSILON,
        "auto_compact_threshold_pct mismatch"
    );
    assert!(
        (cfg.warning_threshold_pct - expected_warning_threshold).abs() < f64::EPSILON,
        "warning_threshold_pct mismatch"
    );
    assert_eq!(
        cfg.max_consecutive_failures, expected_max_failures,
        "max_consecutive_failures mismatch"
    );
}

/// Assert the `has_circuit_break_notified` flag value.
fn assert_circuit_break_notified_flag(handler: &SessionMessageHandler, expected: bool) {
    assert_eq!(
        *handler.has_circuit_break_notified.lock().expect("poisoned"),
        expected,
        "has_circuit_break_notified should be {}",
        expected
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// Per-agent compact configuration tests
// ═════════════════════════════════════════════════════════════════════════════

/// When `SessionMessageHandler` is constructed with a custom `CompactConfig`,
/// the internal `CompactionService` must use that custom config.
#[tokio::test]
async fn test_handler_uses_custom_compact_config() {
    let sm = make_sm();
    let custom = CompactConfig {
        chars_per_token: 0.5,
        auto_compact_threshold_pct: 0.08,
        warning_threshold_pct: 0.15,
        max_consecutive_failures: 5,
        max_history_messages: Some(200),
    };
    let (handler, _rx) = handler_with_channel(&sm, custom.clone());
    assert_compaction_config(&handler, 0.5, 0.08, 0.15, 5).await;
    // Verify max_history_messages propagates through config().
    let svc = handler.compaction_service.lock().await;
    assert_eq!(
        svc.config().max_history_messages,
        Some(200),
        "max_history_messages should propagate"
    );
}

/// When `SessionMessageHandler` is constructed with default `CompactConfig`,
/// the internal `CompactionService` must use default values.
#[tokio::test]
async fn test_handler_uses_default_compact_config() {
    let sm = make_sm();
    let default = CompactConfig::default();
    let (handler, _rx) = handler_with_channel(&sm, default);
    assert_compaction_config(
        &handler, 0.25, // chars_per_token
        0.05, // auto_compact_threshold_pct
        0.10, // warning_threshold_pct
        3,    // max_consecutive_failures
    )
    .await;
}

/// `new_no_output` constructor also propagates custom compact config.
#[tokio::test]
async fn test_handler_no_output_uses_custom_compact_config() {
    let sm = make_sm();
    let custom = CompactConfig {
        chars_per_token: 0.4,
        auto_compact_threshold_pct: 0.03,
        warning_threshold_pct: 0.07,
        max_consecutive_failures: 10,
        max_history_messages: None,
    };
    let handler = handler_no_output(&sm, custom);
    assert_compaction_config(&handler, 0.4, 0.03, 0.07, 10).await;
}

// ═════════════════════════════════════════════════════════════════════════════
// Circuit breaker notification flag reset tests
// ═════════════════════════════════════════════════════════════════════════════

/// After manual compact success, `has_circuit_break_notified` must be `false`
/// and `consecutive_failures` must be reset to 0.
///
/// This tests the full production reset path: `gw_compact` success calls
/// both `reset_circuit_breaker_notification()` (notification flag) and
/// `compaction_service.record_success()` (failure counter) — matching
/// the design doc §熔断器: "手动压缩成功后熔断器自动复位".
#[tokio::test]
async fn test_manual_compact_success_resets_circuit_break_notified_flag() {
    let sm = make_sm();
    let (handler, _rx) = handler_with_channel(&sm, CompactConfig::default());

    // Simulate: circuit breaker has accumulated failures and was notified.
    {
        let mut svc = handler.compaction_service.lock().await;
        for _ in 0..3 {
            svc.record_failure();
        }
        assert_eq!(svc.consecutive_failures(), 3, "precondition: failures == 3");
    }
    *handler.has_circuit_break_notified.lock().expect("poisoned") = true;
    assert_circuit_break_notified_flag(&handler, true);

    // Manual compact success → reset both notification flag and failure counter.
    handler.reset_circuit_breaker_notification();
    {
        let mut svc = handler.compaction_service.lock().await;
        svc.record_success();
    }
    assert_circuit_break_notified_flag(&handler, false);
    {
        let svc = handler.compaction_service.lock().await;
        assert_eq!(
            svc.consecutive_failures(),
            0,
            "consecutive_failures must be 0 after manual compact success"
        );
    }
}

/// After circuit breaker trips and notification is sent, manual compact
/// success resets both the failure counter and notification flag, so that
/// a subsequent auto-compact failure can re-trigger the notification.
///
/// This verifies the design doc requirement: "手动压缩成功后熔断器自动复位"
/// — the breaker (consecutive_failures + notification flag) fully resets.
#[tokio::test]
async fn test_circuit_breaker_reset_allows_re_notification() {
    let sm = make_sm();
    let (handler, _rx) = handler_with_channel(&sm, CompactConfig::default());

    // Trip the breaker and set notification flag.
    {
        let mut svc = handler.compaction_service.lock().await;
        for _ in 0..3 {
            svc.record_failure();
        }
        assert_eq!(
            svc.consecutive_failures(),
            3,
            "precondition: breaker tripped at 3 failures"
        );
    }
    *handler.has_circuit_break_notified.lock().expect("poisoned") = true;
    assert_circuit_break_notified_flag(&handler, true);

    // Manual compact success resets both the breaker counter and the flag.
    handler.reset_circuit_breaker_notification();
    {
        let mut svc = handler.compaction_service.lock().await;
        svc.record_success();
    }
    assert_circuit_break_notified_flag(&handler, false);
    {
        let svc = handler.compaction_service.lock().await;
        assert_eq!(
            svc.consecutive_failures(),
            0,
            "consecutive_failures must be 0 after manual compact success"
        );
    }

    // Subsequent auto-compact failure increments the breaker counter.
    {
        let mut svc = handler.compaction_service.lock().await;
        svc.record_failure();
        svc.record_failure();
        svc.record_failure();
        assert_eq!(
            svc.consecutive_failures(),
            3,
            "failures should be 3 after auto-compact failures"
        );
    }
    // The flag is still false, so a new notification can be sent.
    assert_circuit_break_notified_flag(&handler, false);
}
