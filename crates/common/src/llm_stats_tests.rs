//! Unit tests for [`CacheBreakInfo::format_notification`].
//!
//! Validates that the notification string includes cause keywords,
//! dimension labels, and handles the no-causes fallback.

use super::{
    detect_cache_break, CacheBreakCause, CacheBreakInfo, CacheBreakThresholds, RunningStats,
};

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
