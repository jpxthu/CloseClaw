//! Unit tests for [`CacheBreakInfo::format_notification`].
//!
//! Validates that the notification string includes cause keywords,
//! dimension labels, and handles the no-causes fallback.

use super::{CacheBreakCause, CacheBreakInfo};

/// Helper: build a `CacheBreakInfo` with the given causes.
fn make_info(drop_tokens: u32, drop_ratio: f64, causes: Vec<CacheBreakCause>) -> CacheBreakInfo {
    CacheBreakInfo {
        previous_cache_read: 100_000,
        current_cache_read: 100_000 - drop_tokens,
        drop_tokens,
        drop_ratio,
        causes,
    }
}

#[test]
fn format_notification_single_cause_system_prompt() {
    let info = make_info(5000, 0.05, vec![CacheBreakCause::SystemPromptChanged]);
    let text = info.format_notification();
    assert!(text.contains("system prompt 变更"), "text: {text}");
    assert!(text.contains("system prompt"), "text: {text}");
    assert!(text.contains("5000"), "text: {text}");
}

#[test]
fn format_notification_single_cause_tools() {
    let info = make_info(3000, 0.03, vec![CacheBreakCause::ToolsChanged]);
    let text = info.format_notification();
    assert!(text.contains("工具列表变更"), "text: {text}");
    assert!(text.contains("tools list"), "text: {text}");
}

#[test]
fn format_notification_single_cause_headers() {
    let info = make_info(2500, 0.025, vec![CacheBreakCause::HeadersChanged]);
    let text = info.format_notification();
    assert!(text.contains("请求头变更"), "text: {text}");
    assert!(text.contains("headers"), "text: {text}");
}

#[test]
fn format_notification_single_cause_ttl() {
    let info = make_info(8000, 0.08, vec![CacheBreakCause::TtlExpired]);
    let text = info.format_notification();
    assert!(text.contains("缓存 TTL 过期"), "text: {text}");
    assert!(text.contains("cache ttl"), "text: {text}");
}

#[test]
fn format_notification_single_cause_session_resumed() {
    let info = make_info(10000, 0.10, vec![CacheBreakCause::SessionResumed]);
    let text = info.format_notification();
    assert!(text.contains("会话恢复"), "text: {text}");
    assert!(text.contains("session state"), "text: {text}");
}

#[test]
fn format_notification_single_cause_unknown() {
    let info = make_info(4000, 0.04, vec![CacheBreakCause::Unknown]);
    let text = info.format_notification();
    assert!(text.contains("未知原因"), "text: {text}");
    assert!(text.contains("unknown"), "text: {text}");
}

#[test]
fn format_notification_multiple_causes() {
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
fn format_notification_empty_causes_fallback() {
    let info = make_info(5000, 0.05, vec![]);
    let text = info.format_notification();
    // Empty causes → cause/dimension clauses omitted entirely
    assert!(text.contains("[缓存断点]"), "header present: {text}");
    assert!(text.contains("降幅 5.0%"), "percentage formatted: {text}");
    assert!(text.contains("减少 5000 tokens"), "token count: {text}");
    // Should NOT contain "原因：" or "受影响维度：" when causes is empty
    assert!(!text.contains("原因："), "no cause clause: {text}");
    assert!(
        !text.contains("受影响维度："),
        "no dimension clause: {text}"
    );
}

#[test]
fn format_notification_contains_structured_prefix() {
    let info = make_info(6000, 0.06, vec![CacheBreakCause::TtlExpired]);
    let text = info.format_notification();
    assert!(text.starts_with("[缓存断点]"), "text: {text}");
}

#[test]
fn format_notification_percentage_precision() {
    let info = make_info(3333, 0.03333, vec![CacheBreakCause::Unknown]);
    let text = info.format_notification();
    // drop_ratio * 100 → 3.333% → formatted as {:.1} → "3.3%"
    assert!(text.contains("3.3%"), "text: {text}");
}
