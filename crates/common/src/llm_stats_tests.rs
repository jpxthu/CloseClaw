//! Unit tests for cache anomaly detection and `RunningStats`.
//!
//! Covers `detect_cache_anomaly` (fixed 1024-token tolerance,
//! direction-agnostic absolute deviation, missing-field exemptions),
//! `CacheAnomalyInfo::format_notification`, and `RunningStats`
//! accumulation / state transitions.

use super::{detect_cache_anomaly, CacheAnomalyInfo, RunningStats};
use crate::processor::UnifiedUsage;

/// Design-fixed tolerance value (llm-session-enhancements.md:75).
const TOLERANCE: u32 = 1024;

// ── detect_cache_anomaly: normal path ─────────────────────────────

#[test]
fn detect_anomaly_returns_some_with_all_fields_on_deviation() {
    let info = detect_cache_anomaly(Some(100_000), Some(90_000)).unwrap();
    assert_eq!(info.previous_prompt_tokens, 100_000);
    assert_eq!(info.current_cache_read_tokens, 90_000);
    assert_eq!(info.deviation_tokens, 10_000);
    assert_eq!(info.tolerance_tokens, TOLERANCE);
}

#[test]
fn detect_anomaly_boundary_exactly_tolerance_is_none() {
    assert!(detect_cache_anomaly(Some(100_000), Some(100_000 - TOLERANCE)).is_none());
}

#[test]
fn detect_anomaly_boundary_tolerance_plus_one_is_some() {
    let info = detect_cache_anomaly(Some(100_000), Some(100_000 - TOLERANCE - 1)).unwrap();
    assert_eq!(info.deviation_tokens, TOLERANCE + 1);
    assert_eq!(info.tolerance_tokens, TOLERANCE);
}

#[test]
fn detect_anomaly_direction_agnostic_downward() {
    // cur < prev beyond tolerance → anomaly (prefix invalidated).
    let info = detect_cache_anomaly(Some(50_000), Some(30_000)).unwrap();
    assert_eq!(info.deviation_tokens, 20_000);
    assert_eq!(info.tolerance_tokens, TOLERANCE);
}

#[test]
fn detect_anomaly_direction_agnostic_upward() {
    // cur > prev beyond tolerance is equally an anomaly (absolute value).
    let info = detect_cache_anomaly(Some(50_000), Some(70_000)).unwrap();
    assert_eq!(info.previous_prompt_tokens, 50_000);
    assert_eq!(info.current_cache_read_tokens, 70_000);
    assert_eq!(info.deviation_tokens, 20_000);
}

// ── detect_cache_anomaly: exemptions (missing-field semantics) ────

#[test]
fn detect_anomaly_none_when_both_none() {
    assert!(detect_cache_anomaly(None, None).is_none());
}

#[test]
fn detect_anomaly_none_when_previous_none() {
    assert!(detect_cache_anomaly(None, Some(10_000)).is_none());
}

#[test]
fn detect_anomaly_none_when_current_none() {
    assert!(detect_cache_anomaly(Some(10_000), None).is_none());
}

#[test]
fn detect_anomaly_none_when_previous_zero() {
    // Reference value synthesized to 0 (upstream did not report) → unusable.
    assert!(detect_cache_anomaly(Some(0), Some(50_000)).is_none());
}

#[test]
fn detect_anomaly_genuine_zero_hit_participates() {
    // Some(0) is a real zero hit (whole-round miss) — must be detected,
    // strictly distinguished from None (field absent).
    let info = detect_cache_anomaly(Some(100_000), Some(0)).unwrap();
    assert_eq!(info.current_cache_read_tokens, 0);
    assert_eq!(info.deviation_tokens, 100_000);
}

#[test]
fn detect_anomaly_genuine_zero_hit_within_tolerance_is_none() {
    assert!(detect_cache_anomaly(Some(1_000), Some(0)).is_none());
}

// ── format_notification: five required elements ───────────────────

fn make_info(prev: u32, curr: u32) -> CacheAnomalyInfo {
    CacheAnomalyInfo {
        previous_prompt_tokens: prev,
        current_cache_read_tokens: curr,
        deviation_tokens: prev.abs_diff(curr),
        tolerance_tokens: TOLERANCE,
    }
}

#[test]
fn format_notification_contains_five_elements() {
    let text = make_info(100_000, 90_000).format_notification();
    assert!(text.contains("100000"), "reference value: {text}");
    assert!(text.contains("90000"), "current hit count: {text}");
    assert!(text.contains("10000"), "deviation: {text}");
    assert!(text.contains("1024"), "tolerance: {text}");
    assert!(text.contains("可能原因："), "causes: {text}");
}

#[test]
fn format_notification_contains_cause_list() {
    let text = make_info(100_000, 90_000).format_notification();
    assert!(text.contains("上下文前缀变更"), "cause 1: {text}");
    assert!(text.contains("缓存 TTL 过期"), "cause 2: {text}");
    assert!(text.contains("模型/参数变更"), "cause 3: {text}");
}

#[test]
fn format_notification_structured_prefix() {
    let text = make_info(50_000, 70_000).format_notification();
    assert!(text.starts_with("[缓存命中异常]"), "text: {text}");
}

#[test]
fn format_notification_causes_after_period() {
    let text = make_info(30_000, 90_000).format_notification();
    let causes_pos = text.find("可能原因：").expect("causes marker");
    let period_pos = text.rfind('。').expect("period before causes");
    assert!(
        causes_pos > period_pos,
        "causes should appear after last period: {text}"
    );
}

// ── RunningStats: doc-silent per-call snapshot fields ─────────────
// (last_cache_read_tokens / last_cache_hit_rate: existing advancement
// semantics via the detect-and-update wrapper are preserved.)

#[test]
fn last_cache_read_tokens_none_before_any_accumulate() {
    let stats = RunningStats::new();
    assert_eq!(stats.last_cache_read_tokens, None);
}

#[test]
fn last_cache_read_tokens_set_by_detect_wrapper() {
    let mut stats = RunningStats::new();
    stats.detect_cache_anomaly_and_update(Some(3000), None);
    assert_eq!(stats.last_cache_read_tokens, Some(3000));
}

#[test]
fn last_cache_read_tokens_tracks_latest_value_via_detect() {
    let mut stats = RunningStats::new();
    stats.detect_cache_anomaly_and_update(Some(3000), None);
    assert_eq!(stats.last_cache_read_tokens, Some(3000));
    stats.detect_cache_anomaly_and_update(Some(5000), None);
    assert_eq!(stats.last_cache_read_tokens, Some(5000));
    stats.detect_cache_anomaly_and_update(Some(2000), None);
    assert_eq!(stats.last_cache_read_tokens, Some(2000));
}

#[test]
fn detect_wrapper_tracks_hit_rate() {
    let mut stats = RunningStats::new();
    stats.detect_cache_anomaly_and_update(Some(5000), Some(10000));
    assert!((stats.last_cache_hit_rate.unwrap() - 0.5).abs() < f64::EPSILON);
    let r = stats.detect_cache_anomaly_and_update(Some(4000), Some(10000));
    assert!(r.is_none());
    assert!((stats.last_cache_hit_rate.unwrap() - 0.4).abs() < f64::EPSILON);
}

#[test]
fn detect_wrapper_no_prompt_tokens_no_hit_rate() {
    let mut stats = RunningStats::new();
    stats.detect_cache_anomaly_and_update(Some(5000), None);
    assert!(stats.last_cache_hit_rate.is_none());
}

#[test]
fn detect_wrapper_zero_cache_read_filtered_from_snapshot() {
    // Snapshot fields keep their existing semantics: Some(0) is filtered
    // to None for last_cache_read_tokens / last_cache_hit_rate.
    let mut stats = RunningStats::new();
    stats.detect_cache_anomaly_and_update(Some(0), Some(20000));
    assert_eq!(stats.last_cache_read_tokens, None);
    assert!(stats.last_cache_hit_rate.is_none());
}

// ── RunningStats: detection uses last_prompt_tokens reference ─────

#[test]
fn detect_wrapper_first_call_without_accumulate_is_exempt() {
    // last_prompt_tokens is None → no reference value → no check.
    let mut stats = RunningStats::new();
    let result = stats.detect_cache_anomaly_and_update(Some(90_000), Some(100_000));
    assert!(result.is_none());
    assert!(stats.last_cache_anomaly.is_none());
    assert_eq!(stats.last_cache_read_tokens, Some(90_000));
}

#[test]
fn detect_wrapper_uses_previous_round_reference() {
    let mut stats = RunningStats::new();
    // Round 1: establish the reference via accumulate.
    stats.accumulate(
        &make_usage(100_000, 1_000, Some(101_000), None, None, None),
        None,
    );
    // Round 2: detect BEFORE accumulate — reference is round 1's prompt tokens.
    let info = stats
        .detect_cache_anomaly_and_update(Some(90_000), Some(100_000))
        .expect("deviation beyond tolerance must be detected");
    assert_eq!(info.previous_prompt_tokens, 100_000);
    assert_eq!(info.current_cache_read_tokens, 90_000);
    assert_eq!(info.deviation_tokens, 10_000);
    assert_eq!(stats.last_cache_anomaly(), Some(&info));

    // Round 2 accumulate advances the reference for round 3.
    stats.accumulate(
        &make_usage(100_000, 1_000, Some(101_000), None, None, None),
        None,
    );
    assert_eq!(stats.last_prompt_tokens, Some(100_000));
}

#[test]
fn detect_wrapper_within_tolerance_is_none() {
    let mut stats = RunningStats::new();
    stats.accumulate(
        &make_usage(100_000, 1_000, Some(101_000), None, None, None),
        None,
    );
    let result = stats.detect_cache_anomaly_and_update(Some(100_000 - TOLERANCE), Some(100_000));
    assert!(result.is_none());
    assert!(stats.last_cache_anomaly.is_none());
}

#[test]
fn detect_wrapper_genuine_zero_hit_after_accumulate_detects() {
    let mut stats = RunningStats::new();
    stats.accumulate(
        &make_usage(100_000, 1_000, Some(101_000), None, None, None),
        None,
    );
    // Whole-round miss: real 0 vs reference beyond tolerance → detected.
    let info = stats
        .detect_cache_anomaly_and_update(Some(0), Some(100_000))
        .expect("real zero hit must participate in the check");
    assert_eq!(info.current_cache_read_tokens, 0);
    assert_eq!(info.deviation_tokens, 100_000);
}

// ── RunningStats: accumulate state transitions ────────────────────

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
fn accumulate_advances_last_fields() {
    let mut stats = RunningStats::new();
    stats.accumulate(
        &make_usage(100, 50, Some(150), Some(30), Some(20), None),
        Some("gpt-4"),
    );
    assert_eq!(stats.last_prompt_tokens, Some(100));
    assert_eq!(stats.last_completion_tokens, Some(50));
    assert_eq!(stats.last_call_model.as_deref(), Some("gpt-4"));
    assert!(stats.last_call_at.is_some());

    stats.accumulate(&make_usage(200, 80, Some(280), None, None, None), None);
    assert_eq!(stats.last_prompt_tokens, Some(200));
    assert_eq!(stats.last_completion_tokens, Some(80));
    assert_eq!(stats.last_call_model, None);
}

#[test]
fn accumulate_last_call_at_is_epoch_millis() {
    let mut stats = RunningStats::new();
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    stats.accumulate(&make_usage(100, 50, Some(150), None, None, None), None);
    let after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let at = stats.last_call_at.expect("timestamp set by accumulate");
    assert!(
        at >= before && at <= after,
        "at={at}, window=[{before},{after}]"
    );
}

#[test]
fn test_reset_all_fields_to_initial_values() {
    let mut stats = RunningStats::new();

    // Accumulate some data across multiple calls
    stats.accumulate(
        &make_usage(100, 50, Some(150), Some(30), Some(20), Some(40)),
        Some("m1"),
    );
    stats.accumulate(&make_usage(200, 80, Some(280), Some(60), None, None), None);

    // Use detect wrapper to populate per-call snapshot + anomaly state
    stats.detect_cache_anomaly_and_update(Some(60), Some(200));

    // Verify non-zero state before reset
    assert_eq!(stats.total_prompt_tokens, 300);
    assert_eq!(stats.request_count, 2);
    assert!(stats.last_prompt_tokens.is_some());
    assert!(stats.last_completion_tokens.is_some());
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
    assert!(stats.last_prompt_tokens.is_none());
    assert!(stats.last_completion_tokens.is_none());
    assert!(stats.last_call_at.is_none());
    assert!(stats.last_call_model.is_none());
    assert!(stats.last_cache_read_tokens.is_none());
    assert!(stats.last_cache_hit_rate.is_none());
    assert!(stats.last_cache_anomaly.is_none());
}

#[test]
fn reset_clears_last_cache_anomaly() {
    let mut stats = RunningStats::new();
    stats.accumulate(
        &make_usage(100_000, 1_000, Some(101_000), None, None, None),
        None,
    );
    assert!(stats
        .detect_cache_anomaly_and_update(Some(90_000), Some(100_000))
        .is_some());
    assert!(stats.last_cache_anomaly.is_some());

    stats.reset();
    assert!(stats.last_cache_anomaly.is_none());
    assert!(stats.last_cache_anomaly().is_none());
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

// ── RunningStats: default()/new() behavior tests ──────────────────

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
        stats.last_prompt_tokens.is_none(),
        "last_prompt_tokens should be None"
    );
    assert!(
        stats.last_completion_tokens.is_none(),
        "last_completion_tokens should be None"
    );
    assert!(stats.last_call_at.is_none(), "last_call_at should be None");
    assert!(
        stats.last_call_model.is_none(),
        "last_call_model should be None"
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
        stats.last_cache_anomaly.is_none(),
        "last_cache_anomaly should be None"
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
    stats.accumulate(
        &make_usage(100, 50, Some(150), Some(30), Some(20), None),
        None,
    );
    assert_eq!(stats.total_prompt_tokens, 100);
    assert_eq!(stats.total_completion_tokens, 50);
    assert_eq!(stats.total_tokens, 150);
    assert_eq!(stats.total_cache_read_tokens, 30);
    assert_eq!(stats.total_cache_write_tokens, 20);
    assert_eq!(stats.request_count, 1);

    stats.accumulate(&make_usage(200, 80, Some(280), Some(60), None, None), None);
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
    stats.accumulate(&make_usage(100, 50, Some(150), None, None, Some(100)), None);
    assert_eq!(stats.total_reasoning_tokens, 100);
    assert_eq!(stats.request_count, 1);

    // Second call with reasoning_tokens = None → treated as 0
    stats.accumulate(&make_usage(100, 50, Some(150), None, None, None), None);
    assert_eq!(stats.total_reasoning_tokens, 100);
    assert_eq!(stats.request_count, 2);

    // Third call with reasoning_tokens = Some(200)
    stats.accumulate(&make_usage(100, 50, Some(150), None, None, Some(200)), None);
    assert_eq!(stats.total_reasoning_tokens, 300);
    assert_eq!(stats.request_count, 3);
}

#[test]
fn test_accumulate_all_none_cache_fields() {
    let mut stats = RunningStats::new();
    stats.accumulate(&make_usage(100, 50, Some(150), None, None, None), None);
    assert_eq!(stats.total_cache_read_tokens, 0);
    assert_eq!(stats.total_cache_write_tokens, 0);
}

#[test]
fn test_accumulate_total_none_computed() {
    let mut stats = RunningStats::new();
    stats.accumulate(&make_usage(100, 50, None, None, None, None), None);
    assert_eq!(stats.total_tokens, 150);
}

#[test]
fn test_accumulate_partial_none() {
    let mut stats = RunningStats::new();
    stats.accumulate(&make_usage(100, 50, None, Some(40), None, None), None);
    assert_eq!(stats.total_tokens, 150);
    assert_eq!(stats.total_cache_read_tokens, 40);
    assert_eq!(stats.total_cache_write_tokens, 0);
}

#[test]
fn test_cache_hit_rate_normal() {
    let mut stats = RunningStats::new();
    stats.accumulate(&make_usage(100, 50, Some(150), Some(30), None, None), None);
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
    stats.accumulate(
        &make_usage(100, 50, Some(150), Some(42), Some(10), None),
        None,
    );
    assert_eq!(stats.total_cache_saved(), 42);
}

#[test]
fn test_default_trait() {
    let stats = RunningStats::default();
    assert_eq!(stats.request_count, 0);
    assert_eq!(stats.total_reasoning_tokens, 0);
}
