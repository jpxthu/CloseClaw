//! Unit tests for KV cache prefix stability across multi-turn conversations.
//!
//! Verifies three dimensions:
//! 1. Cache read tokens accumulate correctly across N rounds
//! 2. Cache break detection triggers `tracing::warn!` when thresholds are met
//! 3. Cache hit rate calculation is correct

use super::capture_logs;
use crate::llm_session::ConversationSession;
use closeclaw_common::UnifiedUsage;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Simulate a single LLM round: detect cache break, then accumulate usage.
///
/// This mirrors the session handler flow in
/// `session_handler_announce.rs` (detect → accumulate).
fn simulate_round(session: &mut ConversationSession, usage: &UnifiedUsage) {
    session.detect_cache_break_for_usage(usage.cache_read_tokens, Some(usage.prompt_tokens));
    session.accumulate_usage(usage);
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

/// Test 2: Cache break detection
///
/// First 3 rounds: high cache_read (50 000).
/// Round 4: drops to 10 000 (drop = 40 000, ratio = 80% > 5%, > 2 000).
/// Verify:
/// - `detect_cache_break_for_usage()` returns `Some(CacheBreakInfo)`
/// - `tracing::warn!` is emitted with cache break info
///
/// Uses `#[serial]` plus the shared `capture_logs` helper (thread-local
/// `set_default`) to capture log output on this thread. The previous inline
/// capture used `EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))`:
/// env-driven when `RUST_LOG` parses, warn otherwise. Pinning `WARN` matches
/// that default path's semantics and is now deterministic regardless of
/// `RUST_LOG`.
#[serial_test::serial]
#[tokio::test]
async fn test_cache_break_detection() {
    let (break_info, log_output) = capture_logs(
        || {
            let test_root = TempDir::new().unwrap();
            let mut session = ConversationSession::new(
                "test-cache-break".into(),
                "glm-5".into(),
                test_root.path().to_path_buf(),
            );

            // Rounds 1–3: stable cache at 50 000
            for _ in 1..=3 {
                let usage = make_usage(1000, 200, Some(1200), Some(50_000), None);
                simulate_round(&mut session, &usage);
            }

            // Round 4: cache drops to 10 000 (drop = 40 000, ratio = 80%)
            let drop_usage = make_usage(1000, 200, Some(1200), Some(10_000), None);
            let break_info = session.detect_cache_break_for_usage(
                drop_usage.cache_read_tokens,
                Some(drop_usage.prompt_tokens),
            );

            // Accumulate the dropped round's usage
            session.accumulate_usage(&drop_usage);
            break_info
        },
        tracing::Level::WARN,
    );

    // Assert: detect returns Some with correct break info
    let info = break_info.expect("expected cache break to be detected");
    assert_eq!(info.previous_cache_read, 50_000);
    assert_eq!(info.current_cache_read, 10_000);
    assert_eq!(info.drop_tokens, 40_000);
    assert!(
        (info.drop_ratio - 0.80).abs() < f64::EPSILON,
        "drop_ratio should be 0.80, got {}",
        info.drop_ratio
    );

    // Assert: tracing::warn! was emitted with cache break details
    assert!(
        log_output.contains("cache break"),
        "expected warn log to contain 'cache break', got:\n{log_output}"
    );
    assert!(
        log_output.contains("drop_tokens=40000"),
        "expected warn log to contain drop_tokens=40000, got:\n{log_output}"
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
    for _ in 0..5 {
        let usage = make_usage(1000, 200, Some(1200), Some(300), None);
        simulate_round(&mut session, &usage);
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
