//! Unit tests for KV cache prefix stability across multi-turn conversations.
//!
//! Verifies three dimensions:
//! 1. Cache read tokens accumulate correctly across N rounds
//! 2. Cache anomaly detection triggers `tracing::warn!` when the deviation
//!    between the previous round's prompt tokens and the current cache hit
//!    exceeds the fixed 1024-token tolerance
//! 3. Cache hit rate calculation is correct

use super::capture_logs;
use crate::llm_session::ConversationSession;
use closeclaw_common::{CacheAnomalyInfo, UnifiedUsage};
use tempfile::TempDir;

/// Design-fixed tolerance (llm-session-enhancements.md:75), non-configurable.
const TOLERANCE: u32 = 1024;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Simulate a single LLM round: detect cache anomaly, then accumulate usage.
///
/// This mirrors the session handler flow in
/// `session_handler_announce.rs` (detect → accumulate).
fn simulate_round(session: &mut ConversationSession, usage: &UnifiedUsage) {
    session.detect_cache_anomaly_for_usage(usage.cache_read_tokens, Some(usage.prompt_tokens));
    session.accumulate_usage(usage, None);
}

/// Build a `UnifiedUsage` from the given parameters.
fn make_usage(
    prompt: u32,
    completion: u32,
    total: Option<u32>,
    cache_read: Option<u32>,
    cache_write: Option<u32>,
) -> UnifiedUsage {
    UnifiedUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        reasoning_tokens: None,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
    }
}

/// Build the cache anomaly scenario (issue #3112 场景分层, fixed-1024
/// semantics): a fresh `ConversationSession` under a `TempDir`, two stable
/// rounds with a full cache hit matching the prompt tokens, then a round
/// where the cache read drops to 0 — deviation 100 000 exceeds the fixed
/// tolerance and triggers `detect_cache_anomaly_for_usage`. The dropped
/// round is accumulated afterwards, mirroring detect → accumulate.
///
/// Zero environment capture, so it is a plain fn rather than inlined in
/// the `capture_logs` closure (issue #3112 建议目标).
///
/// Returns the anomaly round's detection result (`Some(CacheAnomalyInfo)`).
fn run_cache_anomaly_scenario() -> Option<CacheAnomalyInfo> {
    let test_root = TempDir::new().unwrap();
    let mut session = ConversationSession::new(
        "test-cache-anomaly".into(),
        "glm-5".into(),
        test_root.path().to_path_buf(),
    );

    // Round 1: first round — no reference value, exempt from checking.
    let stable = make_usage(100_000, 200, Some(100_200), Some(100_000), None);
    simulate_round(&mut session, &stable);

    // Round 2: stable full hit — deviation 0, within tolerance.
    let anomaly_free = session.detect_cache_anomaly_for_usage(Some(100_000), Some(100_000));
    assert!(
        anomaly_free.is_none(),
        "stable round within tolerance must not detect an anomaly"
    );
    session.accumulate_usage(&stable, None);

    // Round 3: cache drops to 0 (whole-round miss) — deviation 100 000
    // exceeds the fixed 1024 tolerance.
    let dropped = make_usage(100_000, 200, Some(100_200), Some(0), None);
    let anomaly = session.detect_cache_anomaly_for_usage(Some(0), Some(dropped.prompt_tokens));

    // Accumulate the dropped round's usage
    session.accumulate_usage(&dropped, None);
    anomaly
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Test 1: Cache hit accumulation
///
/// N rounds (N≥5) each return increasing `cache_read_tokens`.
/// Assert `RunningStats.total_cache_read_tokens` equals the sum of all rounds.
#[tokio::test]
async fn test_cache_hit_accumulation() {
    let n_rounds = 6;
    let test_root = TempDir::new().unwrap();
    let mut session = ConversationSession::new(
        "test-cache-accum".into(),
        "glm-5".into(),
        test_root.path().to_path_buf(),
    );

    // Simulate N rounds of LLM usage accumulation.
    // In production, the session handler calls detect → accumulate after
    // each LLM response. We replicate that flow here.
    for i in 1..=n_rounds {
        let cache_read = i as u32 * 1000;
        let usage = make_usage(500, 100, Some(600), Some(cache_read), None);
        simulate_round(&mut session, &usage);
    }

    let stats = session.stats();

    // Sum of 1000+2000+3000+4000+5000+6000 = 21000
    assert_eq!(
        stats.total_cache_read_tokens, 21_000,
        "total_cache_read_tokens should equal sum of all rounds"
    );
    assert_eq!(
        stats.request_count, n_rounds as u64,
        "request_count should equal number of rounds"
    );
    assert_eq!(
        stats.total_prompt_tokens,
        500 * n_rounds as u64,
        "total_prompt_tokens should accumulate"
    );
}

/// Test 2: Cache anomaly detection under the fixed 1024 tolerance
///
/// Stable rounds keep the cache hit equal to the prompt tokens; the
/// anomaly round drops the hit to 0 (whole-round miss, deviation
/// 100 000 > 1024). Verify:
/// - `detect_cache_anomaly_for_usage()` returns `Some(CacheAnomalyInfo)`
///   with all four fields populated and `tolerance_tokens == 1024`
/// - `tracing::warn!` is emitted with cache anomaly details
///
/// Uses `#[serial]` plus the shared `capture_logs` helper (thread-local
/// `set_default`) to capture log output on this thread. Pinning `WARN`
/// makes the capture deterministic regardless of `RUST_LOG`.
#[serial_test::serial]
#[tokio::test]
async fn test_cache_anomaly_detection() {
    let (anomaly, log_output) = capture_logs(run_cache_anomaly_scenario, tracing::Level::WARN);

    // Assert: detect returns Some with correct anomaly info
    let info = anomaly.expect("expected cache anomaly to be detected");
    assert_eq!(info.previous_prompt_tokens, 100_000);
    assert_eq!(info.current_cache_read_tokens, 0);
    assert_eq!(info.deviation_tokens, 100_000);
    assert_eq!(info.tolerance_tokens, TOLERANCE);

    // Assert: tracing::warn! was emitted with cache anomaly details
    assert!(
        log_output.contains("Cache anomaly"),
        "expected warn log to contain 'Cache anomaly', got:\n{log_output}"
    );
    assert!(
        log_output.contains("deviation_tokens=100000"),
        "expected warn log to contain deviation_tokens=100000, got:\n{log_output}"
    );
    assert!(
        log_output.contains("tolerance_tokens=1024"),
        "expected warn log to contain tolerance_tokens=1024, got:\n{log_output}"
    );
}

/// Test 3: Cache hit rate calculation
///
/// Fixed per-round prompt_tokens and cache_read_tokens.
/// Assert `cache_hit_rate()` = `total_cache_read_tokens / total_prompt_tokens`.
#[tokio::test]
async fn test_cache_hit_rate_calculation() {
    let test_root = TempDir::new().unwrap();
    let mut session = ConversationSession::new(
        "test-cache-rate".into(),
        "glm-5".into(),
        test_root.path().to_path_buf(),
    );

    // 5 rounds: each with prompt=1000, cache_read=300
    // (deviation 700 within tolerance — no anomaly expected).
    for _ in 0..5 {
        let usage = make_usage(1000, 200, Some(1200), Some(300), None);
        let detected = session.detect_cache_anomaly_for_usage(Some(300), Some(1000));
        assert!(
            detected.is_none(),
            "within-tolerance round is not an anomaly"
        );
        session.accumulate_usage(&usage, None);
    }

    let stats = session.stats();

    // total_cache_read_tokens = 5 × 300 = 1500
    // total_prompt_tokens     = 5 × 1000 = 5000
    // expected rate           = 1500 / 5000 = 0.3
    let expected_rate = 1500.0 / 5000.0;
    let actual_rate = stats.cache_hit_rate();

    assert!(
        (actual_rate - expected_rate).abs() < f64::EPSILON,
        "cache_hit_rate should be {expected_rate}, got {actual_rate}"
    );
    assert_eq!(stats.total_cache_read_tokens, 1500);
    assert_eq!(stats.total_prompt_tokens, 5000);
}
