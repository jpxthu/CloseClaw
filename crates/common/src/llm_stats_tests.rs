//! Unit tests for [`CacheBreakInfo::format_notification`].
//!
//! Validates that the notification string includes cause keywords,
//! dimension labels, and handles the no-causes fallback.

use super::{
    detect_cache_break, CacheBreakCause, CacheBreakInfo, CacheBreakThresholds, PendingChanges,
    RunningStats,
};
use crate::processor::UnifiedUsage;

/// Helper: build a `CacheBreakInfo` with the given causes.
fn make_info(drop_tokens: u32, drop_ratio: f64, causes: Vec<CacheBreakCause>) -> CacheBreakInfo {
    CacheBreakInfo {
        previous_cache_read: 100_000,
        current_cache_read: 100_000 - drop_tokens,
        drop_tokens,
        drop_ratio,
        previous_hit_rate: 0.5,
        current_hit_rate: 0.45,
        causes,
    }
}

#[test]
fn test_format_notification_single_cause_system_prompt() {
    let info = make_info(5000, 0.05, vec![CacheBreakCause::SystemPromptChanged]);
    let text = info.format_notification();
    assert!(text.contains("system prompt 变更"), "text: {text}");
    assert!(text.contains("system prompt"), "text: {text}");
    assert!(text.contains("5000"), "text: {text}");
}

#[test]
fn test_format_notification_single_cause_tools() {
    let info = make_info(3000, 0.03, vec![CacheBreakCause::ToolsChanged]);
    let text = info.format_notification();
    assert!(text.contains("工具列表变更"), "text: {text}");
    assert!(text.contains("tools list"), "text: {text}");
}

#[test]
fn test_format_notification_single_cause_headers() {
    let info = make_info(2500, 0.025, vec![CacheBreakCause::HeadersChanged]);
    let text = info.format_notification();
    assert!(text.contains("请求头变更"), "text: {text}");
    assert!(text.contains("headers"), "text: {text}");
}

#[test]
fn test_format_notification_single_cause_ttl() {
    let info = make_info(8000, 0.08, vec![CacheBreakCause::TtlExpired]);
    let text = info.format_notification();
    assert!(text.contains("缓存 TTL 过期"), "text: {text}");
    assert!(text.contains("cache ttl"), "text: {text}");
}

#[test]
fn test_format_notification_single_cause_session_resumed() {
    let info = make_info(10000, 0.10, vec![CacheBreakCause::SessionResumed]);
    let text = info.format_notification();
    assert!(text.contains("会话恢复"), "text: {text}");
    assert!(text.contains("session state"), "text: {text}");
}

#[test]
fn test_format_notification_single_cause_unknown() {
    let info = make_info(4000, 0.04, vec![CacheBreakCause::Unknown]);
    let text = info.format_notification();
    assert!(text.contains("未知原因"), "text: {text}");
    assert!(text.contains("unknown"), "text: {text}");
}

#[test]
fn test_format_notification_multiple_causes() {
    let info = make_info(
        7000,
        0.07,
        vec![
            CacheBreakCause::SystemPromptChanged,
            CacheBreakCause::ToolsChanged,
        ],
    );
    let text = info.format_notification();
    assert!(text.contains("system prompt 变更"), "text: {text}");
    assert!(text.contains("工具列表变更"), "text: {text}");
    // Both dimensions present (English identifiers)
    assert!(text.contains("system prompt"), "text: {text}");
    assert!(text.contains("tools list"), "text: {text}");
    // Joined by Chinese comma
    assert!(text.contains("、"), "text: {text}");
}

#[test]
fn test_format_notification_empty_causes_fallback() {
    let info = make_info(5000, 0.05, vec![]);
    let text = info.format_notification();
    // Empty causes → cause/dimension clauses omitted entirely
    assert!(text.contains("[缓存断点]"), "header present: {text}");
    assert!(text.contains("降幅 5.0%"), "percentage formatted: {text}");
    assert!(text.contains("减少 5000 tokens"), "token count: {text}");
    assert!(text.contains("50.0%"), "previous hit rate: {text}");
    assert!(text.contains("45.0%"), "current hit rate: {text}");
    // Should NOT contain "原因：" or "受影响维度：" when causes is empty
    assert!(!text.contains("原因："), "no cause clause: {text}");
    assert!(
        !text.contains("受影响维度："),
        "no dimension clause: {text}"
    );
}

#[test]
fn test_format_notification_contains_structured_prefix() {
    let info = make_info(6000, 0.06, vec![CacheBreakCause::TtlExpired]);
    let text = info.format_notification();
    assert!(text.starts_with("[缓存断点]"), "text: {text}");
}

#[test]
fn test_format_notification_percentage_precision() {
    let info = make_info(3333, 0.03333, vec![CacheBreakCause::Unknown]);
    let text = info.format_notification();
    // drop_ratio * 100 → 3.333% → formatted as {:.1} → "3.3%"
    assert!(text.contains("3.3%"), "text: {text}");
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

    // Record a fingerprint to populate last_fingerprint and pending_changes
    let tools = vec!["tool_a".to_string()];
    stats.record_fingerprint(Some("old prompt"), Some(&tools), None);
    stats.record_fingerprint(Some("new prompt"), Some(&tools), None);

    // Verify non-zero state before reset
    assert_eq!(stats.total_prompt_tokens, 300);
    assert_eq!(stats.request_count, 2);
    assert!(stats.last_cache_read_tokens.is_some());
    assert!(stats.last_cache_hit_rate.is_some());
    assert!(stats.last_fingerprint.is_some());
    assert!(stats.pending_changes.is_some());

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
    assert!(stats.last_fingerprint.is_none());
    assert!(stats.pending_changes.is_none());
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

// ── record_fingerprint tests ──────────────────────────────────

#[test]
fn record_fingerprint_first_call_no_changes() {
    let mut stats = RunningStats::new();
    let tools = vec!["tool_a".to_string(), "tool_b".to_string()];
    let headers = vec![("content-type", "application/json")];

    // First call: no previous fingerprint → pending_changes is None
    stats.record_fingerprint(Some("You are helpful"), Some(&tools), Some(&headers));
    assert!(stats.pending_changes.is_none());

    // Second call with same fingerprint → all changed flags false
    stats.record_fingerprint(Some("You are helpful"), Some(&tools), Some(&headers));
    let pc = stats.take_pending_changes().unwrap();
    assert!(!pc.system_prompt_changed);
    assert!(!pc.tools_changed);
    assert!(!pc.headers_changed);
    assert!(pc.time_since_last.is_some());
}

#[test]
fn record_fingerprint_detects_system_prompt_change() {
    let mut stats = RunningStats::new();
    let tools = vec!["tool_a".to_string()];

    stats.record_fingerprint(Some("old prompt"), Some(&tools), None);
    assert!(stats.pending_changes.is_none()); // first call

    stats.record_fingerprint(Some("new prompt"), Some(&tools), None);
    let pc = stats.pending_changes.as_ref().unwrap();
    assert!(pc.system_prompt_changed);
    assert!(!pc.tools_changed);
}

#[test]
fn record_fingerprint_detects_tools_change() {
    let mut stats = RunningStats::new();
    let tools_v1 = vec!["tool_a".to_string()];
    let tools_v2 = vec!["tool_a".to_string(), "tool_b".to_string()];

    stats.record_fingerprint(Some("prompt"), Some(&tools_v1), None);
    stats.record_fingerprint(Some("prompt"), Some(&tools_v2), None);

    let pc = stats.pending_changes.as_ref().unwrap();
    assert!(!pc.system_prompt_changed);
    assert!(pc.tools_changed);
}

#[test]
fn record_fingerprint_detects_headers_change() {
    let mut stats = RunningStats::new();
    let tools = vec!["tool_a".to_string()];
    let h1 = vec![("x-api-key", "abc")];
    let h2 = vec![("x-api-key", "xyz")];

    stats.record_fingerprint(Some("prompt"), Some(&tools), Some(&h1));
    stats.record_fingerprint(Some("prompt"), Some(&tools), Some(&h2));

    let pc = stats.pending_changes.as_ref().unwrap();
    assert!(pc.headers_changed);
    assert!(!pc.system_prompt_changed);
}

#[test]
fn record_fingerprint_none_inputs_no_panic() {
    let mut stats = RunningStats::new();
    stats.record_fingerprint(None, None, None);
    assert!(stats.pending_changes.is_none()); // first call

    stats.record_fingerprint(None, None, None);
    let pc = stats.take_pending_changes().unwrap();
    assert!(!pc.system_prompt_changed);
    assert!(!pc.tools_changed);
    assert!(!pc.headers_changed);
}

#[test]
fn record_fingerprint_empty_tools_no_panic() {
    let mut stats = RunningStats::new();
    let empty: Vec<String> = vec![];
    stats.record_fingerprint(None, Some(&empty), None);
    assert!(stats.pending_changes.is_none()); // first call

    stats.record_fingerprint(None, Some(&empty), None);
    let pc = stats.take_pending_changes().unwrap();
    assert!(!pc.tools_changed);
}

#[test]
fn record_fingerprint_empty_headers_no_panic() {
    let mut stats = RunningStats::new();
    let empty_headers: [(&str, &str); 0] = [];
    stats.record_fingerprint(None, None, Some(&empty_headers));
    assert!(stats.pending_changes.is_none()); // first call

    stats.record_fingerprint(None, None, Some(&empty_headers));
    let pc = stats.take_pending_changes().unwrap();
    assert!(!pc.headers_changed);
}

#[test]
fn record_fingerprint_three_calls_mixed_changes() {
    let mut stats = RunningStats::new();
    let tools = vec!["tool_a".to_string()];

    // call 1: baseline
    stats.record_fingerprint(Some("prompt_v1"), Some(&tools), None);
    assert!(stats.pending_changes.is_none()); // first call

    // call 2: system_prompt changed
    stats.record_fingerprint(Some("prompt_v2"), Some(&tools), None);
    let pc = stats.pending_changes.as_ref().unwrap();
    assert!(pc.system_prompt_changed);
    assert!(!pc.tools_changed);

    // call 3: tools changed, system_prompt reverted
    let tools2 = vec!["tool_a".to_string(), "tool_b".to_string()];
    stats.record_fingerprint(Some("prompt_v1"), Some(&tools2), None);
    let pc = stats.pending_changes.as_ref().unwrap();
    assert!(pc.system_prompt_changed); // reverted = changed
    assert!(pc.tools_changed);
}

#[test]
fn take_pending_changes_clears_field() {
    let mut stats = RunningStats::new();
    let tools = vec!["tool_a".to_string()];
    stats.record_fingerprint(Some("p1"), Some(&tools), None);
    stats.record_fingerprint(Some("p2"), Some(&tools), None);

    let pc1 = stats.take_pending_changes();
    assert!(pc1.is_some());
    assert!(pc1.unwrap().system_prompt_changed);

    let pc2 = stats.take_pending_changes();
    assert!(pc2.is_none());
}

// ── cache break attribution tests ────────────────────────────

#[test]
fn attribution_system_prompt_changed_triggers_cause() {
    let mut stats = RunningStats::new();
    stats.last_cache_read_tokens = Some(100_000);
    let tools = vec!["tool_a".to_string()];

    stats.record_fingerprint(Some("old prompt"), Some(&tools), None);
    stats.record_fingerprint(Some("new prompt"), Some(&tools), None);

    let info = stats
        .detect_cache_break_and_update(Some(90_000), None)
        .unwrap();
    assert!(info.causes.contains(&CacheBreakCause::SystemPromptChanged));
}

#[test]
fn attribution_tools_changed_triggers_cause() {
    let mut stats = RunningStats::new();
    stats.last_cache_read_tokens = Some(100_000);
    let tools_v1 = vec!["tool_a".to_string()];
    let tools_v2 = vec!["tool_a".to_string(), "tool_b".to_string()];

    stats.record_fingerprint(Some("prompt"), Some(&tools_v1), None);
    stats.record_fingerprint(Some("prompt"), Some(&tools_v2), None);

    let info = stats
        .detect_cache_break_and_update(Some(90_000), None)
        .unwrap();
    assert!(info.causes.contains(&CacheBreakCause::ToolsChanged));
}

#[test]
fn attribution_headers_changed_triggers_cause() {
    let mut stats = RunningStats::new();
    stats.last_cache_read_tokens = Some(100_000);
    let tools = vec!["tool_a".to_string()];
    let h1 = vec![("x-api-key", "abc")];
    let h2 = vec![("x-api-key", "xyz")];

    stats.record_fingerprint(Some("prompt"), Some(&tools), Some(&h1));
    stats.record_fingerprint(Some("prompt"), Some(&tools), Some(&h2));

    let info = stats
        .detect_cache_break_and_update(Some(90_000), None)
        .unwrap();
    assert!(info.causes.contains(&CacheBreakCause::HeadersChanged));
}

#[test]
fn attribution_ttl_expired_triggers_cause() {
    let mut stats = RunningStats::new();
    stats.last_cache_read_tokens = Some(100_000);
    // Directly set pending_changes with a duration exceeding TTL
    stats.pending_changes = Some(PendingChanges {
        system_prompt_changed: false,
        tools_changed: false,
        headers_changed: false,
        time_since_last: Some(std::time::Duration::from_secs(600)),
    });

    let info = stats
        .detect_cache_break_and_update(Some(90_000), None)
        .unwrap();
    assert!(info.causes.contains(&CacheBreakCause::TtlExpired));
}

#[test]
fn attribution_no_pending_changes_yields_unknown() {
    let mut stats = RunningStats::new();
    stats.last_cache_read_tokens = Some(100_000);
    // Set request_count > 0 to avoid SessionResumed trigger
    stats.request_count = 1;
    // No pending_changes recorded

    let info = stats
        .detect_cache_break_and_update(Some(90_000), None)
        .unwrap();
    assert!(info.causes.contains(&CacheBreakCause::Unknown));
}

#[test]
fn attribution_no_cache_break_no_causes() {
    let mut stats = RunningStats::new();
    stats.last_cache_read_tokens = Some(100_000);
    let tools = vec!["tool_a".to_string()];

    stats.record_fingerprint(Some("old prompt"), Some(&tools), None);
    stats.record_fingerprint(Some("new prompt"), Some(&tools), None);

    // Drop below threshold → no cache break → no causes
    let result = stats.detect_cache_break_and_update(Some(99_000), None);
    assert!(result.is_none());
}

#[test]
fn attribution_after_take_pending_correct() {
    let mut stats = RunningStats::new();
    stats.last_cache_read_tokens = Some(100_000);
    stats.request_count = 1;
    let tools = vec!["tool_a".to_string()];

    stats.record_fingerprint(Some("old"), Some(&tools), None);
    stats.record_fingerprint(Some("new"), Some(&tools), None);

    // Take pending changes before detection
    let _taken = stats.take_pending_changes();
    assert!(stats.pending_changes.is_none());

    // After take, no pending → attribution yields Unknown
    let info = stats
        .detect_cache_break_and_update(Some(90_000), None)
        .unwrap();
    assert!(info.causes.contains(&CacheBreakCause::Unknown));
}

#[test]
fn attribution_session_resumed_on_first_accumulate() {
    let mut stats = RunningStats::new();
    stats.last_cache_read_tokens = Some(100_000);
    // request_count == 0 + last_cache_read_tokens.is_some() → SessionResumed

    let info = stats
        .detect_cache_break_and_update(Some(90_000), None)
        .unwrap();
    assert!(info.causes.contains(&CacheBreakCause::SessionResumed));
}
