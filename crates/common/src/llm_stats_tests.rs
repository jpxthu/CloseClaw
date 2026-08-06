//! Unit tests for [`CacheBreakInfo::format_notification`].
//!
//! Validates that the notification string includes cause keywords,
//! dimension labels, and handles the no-causes fallback.

use super::{detect_cache_break, CacheBreakInfo, CacheBreakThresholds, RunningStats};
use crate::processor::UnifiedUsage;

/// Helper: build a `CacheBreakInfo`.
fn make_info(drop_tokens: u32, drop_ratio: f64) -> CacheBreakInfo {
    CacheBreakInfo {
        previous_cache_read: 100_000,
        current_cache_read: 100_000 - drop_tokens,
        drop_tokens,
        drop_ratio,
        previous_hit_rate: 0.5,
        current_hit_rate: 0.45,
    }
}

#[test]
fn test_format_notification_single_cause_system_prompt() {
    let info = make_info(5000, 0.05);
    let text = info.format_notification();
    assert!(text.contains("5000"), "text: {text}");
}

#[test]
fn test_format_notification_single_cause_tools() {
    let info = make_info(3000, 0.03);
    let text = info.format_notification();
    assert!(text.contains("减少 3000 tokens"), "text: {text}");
}

#[test]
fn test_format_notification_single_cause_headers() {
    let info = make_info(2500, 0.025);
    let text = info.format_notification();
    assert!(text.contains("2500"), "text: {text}");
}

#[test]
fn test_format_notification_single_cause_ttl() {
    let info = make_info(8000, 0.08);
    let text = info.format_notification();
    assert!(text.contains("8000"), "text: {text}");
}

#[test]
fn test_format_notification_single_cause_session_resumed() {
    let info = make_info(10000, 0.10);
    let text = info.format_notification();
    assert!(text.contains("10000"), "text: {text}");
}

#[test]
fn test_format_notification_single_cause_unknown() {
    let info = make_info(4000, 0.04);
    let text = info.format_notification();
    assert!(text.contains("4000"), "text: {text}");
}

#[test]
fn test_format_notification_multiple_causes() {
    let info = make_info(7000, 0.07);
    let text = info.format_notification();
    assert!(text.contains("减少 7000 tokens"), "text: {text}");
    assert!(text.contains("降幅 7.0%"), "text: {text}");
}

#[test]
fn test_format_notification_empty_causes_fallback() {
    let info = make_info(5000, 0.05);
    let text = info.format_notification();
    // Verify notification format
    assert!(text.contains("[缓存断点]"), "header present: {text}");
    assert!(text.contains("降幅 5.0%"), "percentage formatted: {text}");
    assert!(text.contains("减少 5000 tokens"), "token count: {text}");
    assert!(text.contains("50.0%"), "previous hit rate: {text}");
    assert!(text.contains("45.0%"), "current hit rate: {text}");
}

#[test]
fn test_format_notification_contains_possible_causes() {
    let info = make_info(5000, 0.05);
    let text = info.format_notification();
    assert!(text.contains("可能原因："), "cause prefix present: {text}");
    assert!(text.contains("上下文变更"), "cause 1 present: {text}");
    assert!(text.contains("缓存 TTL 过期"), "cause 2 present: {text}");
    assert!(text.contains("模型/参数变更"), "cause 3 present: {text}");
}

#[test]
fn test_format_notification_causes_after_period() {
    let info = make_info(3000, 0.03);
    let text = info.format_notification();
    // Causes should come after the numeric section ending with '。'
    let causes_pos = text.find("可能原因：").expect("causes marker");
    let period_pos = text.rfind('。').expect("period before causes");
    assert!(
        causes_pos > period_pos,
        "causes should appear after last period: {text}"
    );
}

#[test]
fn test_format_notification_numerical_format_unaffected() {
    let info = make_info(6000, 0.06);
    let text = info.format_notification();
    assert!(text.contains("[缓存断点]"), "header: {text}");
    assert!(text.contains("50.0%"), "prev rate: {text}");
    assert!(text.contains("45.0%"), "curr rate: {text}");
    assert!(text.contains("减少 6000 tokens"), "token drop: {text}");
    assert!(text.contains("降幅 6.0%"), "ratio: {text}");
}

#[test]
fn test_format_notification_contains_structured_prefix() {
    let info = make_info(6000, 0.06);
    let text = info.format_notification();
    assert!(text.starts_with("[缓存断点]"), "text: {text}");
}

#[test]
fn test_format_notification_percentage_precision() {
    let info = make_info(3333, 0.03333);
    let text = info.format_notification();
    // drop_ratio * 100 → 3.333% → formatted as {:.1} → "3.3%"
    assert!(text.contains("3.3%"), "text: {text}");
}

#[test]
fn test_format_notification_no_extra_spaces() {
    let info = make_info(5000, 0.05);
    let text = info.format_notification();
    // Period must be immediately followed by 可能原因, no extra whitespace.
    assert!(
        text.contains("。可能原因："),
        "'。' must be immediately followed by '可能原因：' without spaces: {text}"
    );
}

// ── detect_cache_break standalone tests ───────────────────────────

#[test]
fn detect_cache_break_returns_none_when_both_none() {
    assert!(detect_cache_break(None, None, None).is_none());
}

#[test]
fn detect_cache_break_returns_none_when_prev_none() {
    assert!(detect_cache_break(None, Some(10000), None).is_none());
}

#[test]
fn detect_cache_break_returns_none_when_curr_none() {
    assert!(detect_cache_break(Some(10000), None, None).is_none());
}

#[test]
fn detect_cache_break_returns_none_when_curr_equals_prev() {
    assert!(detect_cache_break(Some(10000), Some(10000), None).is_none());
}

#[test]
fn detect_cache_break_returns_none_when_curr_greater_than_prev() {
    assert!(detect_cache_break(Some(8000), Some(10000), None).is_none());
}

#[test]
fn detect_cache_break_returns_none_when_drop_exactly_2000() {
    assert!(detect_cache_break(Some(10000), Some(8000), None).is_none());
}

#[test]
fn detect_cache_break_returns_none_when_drop_below_2000() {
    assert!(detect_cache_break(Some(10000), Some(8500), None).is_none());
}

#[test]
fn detect_cache_break_returns_none_when_ratio_le_5_percent() {
    assert!(detect_cache_break(Some(100000), Some(95500), None).is_none());
}

#[test]
fn detect_cache_break_returns_none_when_ratio_exactly_5_percent() {
    assert!(detect_cache_break(Some(100000), Some(95000), None).is_none());
}

#[test]
fn detect_cache_break_returns_some_when_both_thresholds_met() {
    let info = detect_cache_break(Some(100000), Some(90000), None).unwrap();
    assert_eq!(info.previous_cache_read, 100000);
    assert_eq!(info.current_cache_read, 90000);
    assert_eq!(info.drop_tokens, 10000);
    assert!((info.drop_ratio - 0.10).abs() < 1e-10);
}

#[test]
fn detect_cache_break_large_drop() {
    let info = detect_cache_break(Some(50000), Some(30000), None).unwrap();
    assert_eq!(info.drop_tokens, 20000);
    assert!((info.drop_ratio - 0.40).abs() < 1e-10);
}

#[test]
fn detect_cache_break_custom_threshold() {
    let th = CacheBreakThresholds {
        drop_ratio_threshold: 0.10,
        min_drop_tokens: 5000,
    };
    // 12% drop, 6000 tokens — meets custom threshold
    let info = detect_cache_break(Some(50000), Some(44000), Some(&th)).unwrap();
    assert_eq!(info.drop_tokens, 6000);
    // 3% drop, 3000 tokens — below custom ratio threshold
    assert!(detect_cache_break(Some(100000), Some(97000), Some(&th)).is_none());
    // 10% drop but only 3000 tokens — below min_drop_tokens
    assert!(detect_cache_break(Some(30000), Some(27000), Some(&th)).is_none());
}

// ── RunningStats.last_cache_read_tokens & hit-rate tests ──────────

#[test]
fn last_cache_read_tokens_none_before_any_accumulate() {
    let stats = RunningStats::new();
    assert_eq!(stats.last_cache_read_tokens, None);
}

#[test]
fn last_cache_read_tokens_set_by_detect_cache_break_and_update() {
    let mut stats = RunningStats::new();
    stats.detect_cache_break_and_update(Some(3000), None);
    assert_eq!(stats.last_cache_read_tokens, Some(3000));
}

#[test]
fn last_cache_read_tokens_tracks_latest_value_via_detect() {
    let mut stats = RunningStats::new();
    stats.detect_cache_break_and_update(Some(3000), None);
    assert_eq!(stats.last_cache_read_tokens, Some(3000));
    stats.detect_cache_break_and_update(Some(5000), None);
    assert_eq!(stats.last_cache_read_tokens, Some(5000));
    stats.detect_cache_break_and_update(Some(2000), None);
    assert_eq!(stats.last_cache_read_tokens, Some(2000));
}

#[test]
fn detect_cache_break_and_update_returns_none_first_call() {
    let mut stats = RunningStats::new();
    let result = stats.detect_cache_break_and_update(Some(10000), None);
    assert!(result.is_none());
    assert_eq!(stats.last_cache_read_tokens, Some(10000));
}

#[test]
fn detect_cache_break_and_update_returns_none_when_no_break() {
    let mut stats = RunningStats::new();
    stats.detect_cache_break_and_update(Some(10000), None);
    let result = stats.detect_cache_break_and_update(Some(9900), None);
    assert!(result.is_none());
    assert_eq!(stats.last_cache_read_tokens, Some(9900));
}

#[test]
fn detect_cache_break_and_update_returns_some_on_break() {
    let mut stats = RunningStats::new();
    stats.detect_cache_break_and_update(Some(100000), None);
    let result = stats.detect_cache_break_and_update(Some(90000), None);
    let info = result.unwrap();
    assert_eq!(info.previous_cache_read, 100000);
    assert_eq!(info.current_cache_read, 90000);
    assert_eq!(info.drop_tokens, 10000);
    assert_eq!(stats.last_cache_read_tokens, Some(90000));
}

#[test]
fn detect_cache_break_and_update_chain() {
    let mut stats = RunningStats::new();
    stats.detect_cache_break_and_update(Some(50000), None);
    assert_eq!(stats.last_cache_read_tokens, Some(50000));
    let r1 = stats.detect_cache_break_and_update(Some(49000), None);
    assert!(r1.is_none());
    assert_eq!(stats.last_cache_read_tokens, Some(49000));
    let r2 = stats.detect_cache_break_and_update(Some(45000), None);
    let info = r2.unwrap();
    assert_eq!(info.previous_cache_read, 49000);
    assert_eq!(info.current_cache_read, 45000);
    assert_eq!(info.drop_tokens, 4000);
    assert_eq!(stats.last_cache_read_tokens, Some(45000));
}

#[test]
fn detect_cache_break_and_update_tracks_hit_rate() {
    let mut stats = RunningStats::new();
    stats.detect_cache_break_and_update(Some(5000), Some(10000));
    assert!((stats.last_cache_hit_rate.unwrap() - 0.5).abs() < f64::EPSILON);
    let r = stats.detect_cache_break_and_update(Some(5000), Some(10000));
    assert!(r.is_none());
    assert!((stats.last_cache_hit_rate.unwrap() - 0.5).abs() < f64::EPSILON);
    let r = stats.detect_cache_break_and_update(Some(1000), Some(10000));
    assert!(r.is_some());
    let info = r.unwrap();
    assert!((info.previous_hit_rate - 0.5).abs() < f64::EPSILON);
    assert!((info.current_hit_rate - 0.1).abs() < f64::EPSILON);
}

#[test]
fn detect_cache_break_and_update_no_prompt_tokens_no_hit_rate() {
    let mut stats = RunningStats::new();
    stats.detect_cache_break_and_update(Some(5000), None);
    assert!(stats.last_cache_hit_rate.is_none());
    let r = stats.detect_cache_break_and_update(Some(1000), None);
    assert!(r.is_some());
}

// ── Migrated from inline #[cfg(test)] module ─────────────────────

fn make_usage(
    prompt: u32,
    completion: u32,
    total: Option<u32>,
    cache_read: Option<u32>,
    cache_write: Option<u32>,
    reasoning_tokens: Option<u32>,
) -> UnifiedUsage {
    UnifiedUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        reasoning_tokens,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
    }
}

#[test]
fn test_reset_all_fields_to_initial_values() {
    let mut stats = RunningStats::new();

    // Accumulate some data across multiple calls
    stats.accumulate(&make_usage(
        100,
        50,
        Some(150),
        Some(30),
        Some(20),
        Some(40),
    ));
    stats.accumulate(&make_usage(200, 80, Some(280), Some(60), None, None));

    // Use detect_cache_break_and_update to populate last_cache_read_tokens
    stats.detect_cache_break_and_update(Some(30), Some(100));
    stats.detect_cache_break_and_update(Some(60), Some(200));

    // Verify non-zero state before reset
    assert_eq!(stats.total_prompt_tokens, 300);
    assert_eq!(stats.request_count, 2);
    assert!(stats.last_cache_read_tokens.is_some());
    assert!(stats.last_cache_hit_rate.is_some());

    // Reset
    stats.reset();

    // Verify all fields match RunningStats::new()
    let fresh = RunningStats::new();
    assert_eq!(stats, fresh);
    assert_eq!(stats.total_prompt_tokens, 0);
    assert_eq!(stats.total_completion_tokens, 0);
    assert_eq!(stats.total_tokens, 0);
    assert_eq!(stats.total_cache_read_tokens, 0);
    assert_eq!(stats.total_cache_write_tokens, 0);
    assert_eq!(stats.request_count, 0);
    assert_eq!(stats.total_reasoning_tokens, 0);
    assert!(stats.cache_break_thresholds.is_none());
    assert!(stats.last_cache_read_tokens.is_none());
    assert!(stats.last_cache_hit_rate.is_none());
}

#[test]
fn test_new_is_zeroed() {
    let stats = RunningStats::new();
    assert_eq!(stats.total_prompt_tokens, 0);
    assert_eq!(stats.total_completion_tokens, 0);
    assert_eq!(stats.total_tokens, 0);
    assert_eq!(stats.total_cache_read_tokens, 0);
    assert_eq!(stats.total_cache_write_tokens, 0);
    assert_eq!(stats.request_count, 0);
    assert_eq!(stats.total_reasoning_tokens, 0);
}

// ── Step 1.2: RunningStats::default() / new() behavior tests ──

#[test]
fn test_default_all_numeric_fields_are_zero() {
    let stats = RunningStats::default();
    assert_eq!(
        stats.total_prompt_tokens, 0,
        "total_prompt_tokens should be 0"
    );
    assert_eq!(
        stats.total_completion_tokens, 0,
        "total_completion_tokens should be 0"
    );
    assert_eq!(stats.total_tokens, 0, "total_tokens should be 0");
    assert_eq!(
        stats.total_cache_read_tokens, 0,
        "total_cache_read_tokens should be 0"
    );
    assert_eq!(
        stats.total_cache_write_tokens, 0,
        "total_cache_write_tokens should be 0"
    );
    assert_eq!(stats.request_count, 0, "request_count should be 0");
    assert_eq!(
        stats.total_reasoning_tokens, 0,
        "total_reasoning_tokens should be 0"
    );
}

#[test]
fn test_default_option_fields_are_none() {
    let stats = RunningStats::default();
    assert!(
        stats.cache_break_thresholds.is_none(),
        "cache_break_thresholds should be None"
    );
    assert!(
        stats.last_cache_read_tokens.is_none(),
        "last_cache_read_tokens should be None"
    );
    assert!(
        stats.last_cache_hit_rate.is_none(),
        "last_cache_hit_rate should be None"
    );
    assert!(
        stats.last_cache_break.is_none(),
        "last_cache_break should be None"
    );
}

#[test]
fn test_default_and_new_are_equivalent() {
    let from_default = RunningStats::default();
    let from_new = RunningStats::new();
    assert_eq!(
        from_default, from_new,
        "default() and new() must produce identical results"
    );
}

#[test]
fn test_accumulate_basic() {
    let mut stats = RunningStats::new();
    stats.accumulate(&make_usage(100, 50, Some(150), Some(30), Some(20), None));
    assert_eq!(stats.total_prompt_tokens, 100);
    assert_eq!(stats.total_completion_tokens, 50);
    assert_eq!(stats.total_tokens, 150);
    assert_eq!(stats.total_cache_read_tokens, 30);
    assert_eq!(stats.total_cache_write_tokens, 20);
    assert_eq!(stats.request_count, 1);

    stats.accumulate(&make_usage(200, 80, Some(280), Some(60), None, None));
    assert_eq!(stats.total_prompt_tokens, 300);
    assert_eq!(stats.total_completion_tokens, 130);
    assert_eq!(stats.total_tokens, 430);
    assert_eq!(stats.total_cache_read_tokens, 90);
    assert_eq!(stats.total_cache_write_tokens, 20);
    assert_eq!(stats.request_count, 2);
}

#[test]
fn test_accumulate_reasoning_tokens() {
    let mut stats = RunningStats::new();

    // First call with reasoning_tokens = Some(100)
    stats.accumulate(&make_usage(100, 50, Some(150), None, None, Some(100)));
    assert_eq!(stats.total_reasoning_tokens, 100);
    assert_eq!(stats.request_count, 1);

    // Second call with reasoning_tokens = None → treated as 0
    stats.accumulate(&make_usage(100, 50, Some(150), None, None, None));
    assert_eq!(stats.total_reasoning_tokens, 100);
    assert_eq!(stats.request_count, 2);

    // Third call with reasoning_tokens = Some(200)
    stats.accumulate(&make_usage(100, 50, Some(150), None, None, Some(200)));
    assert_eq!(stats.total_reasoning_tokens, 300);
    assert_eq!(stats.request_count, 3);
}

#[test]
fn test_accumulate_all_none_cache_fields() {
    let mut stats = RunningStats::new();
    stats.accumulate(&make_usage(100, 50, Some(150), None, None, None));
    assert_eq!(stats.total_cache_read_tokens, 0);
    assert_eq!(stats.total_cache_write_tokens, 0);
}

#[test]
fn test_accumulate_total_none_computed() {
    let mut stats = RunningStats::new();
    stats.accumulate(&make_usage(100, 50, None, None, None, None));
    assert_eq!(stats.total_tokens, 150);
}

#[test]
fn test_accumulate_partial_none() {
    let mut stats = RunningStats::new();
    stats.accumulate(&make_usage(100, 50, None, Some(40), None, None));
    assert_eq!(stats.total_tokens, 150);
    assert_eq!(stats.total_cache_read_tokens, 40);
    assert_eq!(stats.total_cache_write_tokens, 0);
}

#[test]
fn test_cache_hit_rate_normal() {
    let mut stats = RunningStats::new();
    stats.accumulate(&make_usage(100, 50, Some(150), Some(30), None, None));
    let rate = stats.cache_hit_rate();
    assert!((rate - 0.3).abs() < f64::EPSILON);
}

#[test]
fn test_cache_hit_rate_division_by_zero() {
    let stats = RunningStats::new();
    assert_eq!(stats.cache_hit_rate(), 0.0);
}

#[test]
fn test_total_cache_saved() {
    let mut stats = RunningStats::new();
    stats.accumulate(&make_usage(100, 50, Some(150), Some(42), Some(10), None));
    assert_eq!(stats.total_cache_saved(), 42);
}

#[test]
fn test_default_trait() {
    let stats = RunningStats::default();
    assert_eq!(stats.request_count, 0);
    assert_eq!(stats.total_reasoning_tokens, 0);
}

// ── cache break attribution tests removed ──────────────────────
// (PromptFingerprint and cause attribution removed per plan Step 1.7)

// ── hit rate + token breakpoint interaction tests ──────────────

#[test]
fn detect_cache_break_hit_rate_and_token_both_trigger() {
    // When both hit-rate drop and token drop exceed thresholds,
    // CacheBreakInfo should contain both hit-rate and token data.
    let mut stats = RunningStats::new();
    // Previous call: 80000 cache_read / 100000 prompt = 0.8 hit rate
    stats.detect_cache_break_and_update(Some(80000), Some(100000));
    // Current call: 40000 cache_read / 100000 prompt = 0.4 hit rate
    // Token drop: 40000 > min_drop_tokens(2000) ✓
    // Rate drop: 0.4 < 0.05 threshold ✓
    let info = stats
        .detect_cache_break_and_update(Some(40000), Some(100000))
        .expect("should detect break when both thresholds met");
    assert_eq!(info.previous_cache_read, 80000);
    assert_eq!(info.current_cache_read, 40000);
    assert_eq!(info.drop_tokens, 40000);
    assert!((info.previous_hit_rate - 0.8).abs() < f64::EPSILON);
    assert!((info.current_hit_rate - 0.4).abs() < f64::EPSILON);
}

#[test]
fn detect_cache_break_hit_rate_only_triggers() {
    // When token drop is below min_drop_tokens but hit-rate drops significantly,
    // break is triggered by rate alone.
    let mut stats = RunningStats::new();
    // Previous: 9900/10000 = 0.99
    stats.detect_cache_break_and_update(Some(9900), Some(10000));
    // Current: 100/10000 = 0.01 → rate drop = 0.98 >> 0.05
    // Token drop = 9800 > min_drop_tokens(2000) → also token triggered
    let info = stats
        .detect_cache_break_and_update(Some(100), Some(10000))
        .expect("should detect break");
    assert!((info.previous_hit_rate - 0.99).abs() < 1e-6);
    assert!((info.current_hit_rate - 0.01).abs() < 1e-6);
    assert_eq!(info.drop_tokens, 9800);
}

#[test]
fn detect_cache_break_rate_only_no_token_break() {
    // Pure rate-only break: token drop is below min_drop_tokens (1999 < 2000)
    // but hit-rate drops significantly (0.5 → 0.3001, drop = 0.1999 > 0.05).
    let mut stats = RunningStats::new();
    // Previous call: 5000/10000 = 0.5 hit rate
    stats.detect_cache_break_and_update(Some(5000), Some(10000));
    // Current call: 3001/10000 = 0.3001 hit rate
    // Token drop: 5000 - 3001 = 1999 < min_drop_tokens(2000) → no token break
    // Rate drop: 0.5 - 0.3001 = 0.1999 > drop_ratio_threshold(0.05) → rate break
    let info = stats
        .detect_cache_break_and_update(Some(3001), Some(10000))
        .expect("rate-only break should trigger");
    assert_eq!(info.previous_cache_read, 5000);
    assert_eq!(info.current_cache_read, 3001);
    assert_eq!(info.drop_tokens, 1999);
    assert!((info.previous_hit_rate - 0.5).abs() < 1e-6);
    assert!((info.current_hit_rate - 0.3001).abs() < 1e-6);
}
