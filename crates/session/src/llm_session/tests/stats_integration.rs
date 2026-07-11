//! Integration tests for ConversationSession + RunningStats.

use super::*;
use closeclaw_common::UnifiedUsage;

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

#[test]
fn test_session_stats_initial_state() {
    let session = ConversationSession::new("sess_stats_init".into(), "gpt-4o".into(), tmp_path());
    let stats = session.stats();
    assert_eq!(stats.total_prompt_tokens, 0);
    assert_eq!(stats.total_completion_tokens, 0);
    assert_eq!(stats.total_tokens, 0);
    assert_eq!(stats.total_cache_read_tokens, 0);
    assert_eq!(stats.total_cache_write_tokens, 0);
    assert_eq!(stats.request_count, 0);
}

#[test]
fn test_session_accumulate_usage_single_call() {
    let mut session =
        ConversationSession::new("sess_stats_single".into(), "gpt-4o".into(), tmp_path());
    session.accumulate_usage(&make_usage(100, 50, Some(150), Some(30), Some(20)));
    let stats = session.stats();
    assert_eq!(stats.total_prompt_tokens, 100);
    assert_eq!(stats.total_completion_tokens, 50);
    assert_eq!(stats.total_tokens, 150);
    assert_eq!(stats.total_cache_read_tokens, 30);
    assert_eq!(stats.total_cache_write_tokens, 20);
    assert_eq!(stats.request_count, 1);
}

#[test]
fn test_session_accumulate_usage_multiple_calls() {
    let mut session =
        ConversationSession::new("sess_stats_multi".into(), "gpt-4o".into(), tmp_path());
    session.accumulate_usage(&make_usage(100, 50, Some(150), Some(30), Some(20)));
    session.accumulate_usage(&make_usage(200, 80, Some(280), Some(60), None));
    session.accumulate_usage(&make_usage(50, 20, None, None, Some(10)));

    let stats = session.stats();
    assert_eq!(stats.total_prompt_tokens, 350);
    assert_eq!(stats.total_completion_tokens, 150);
    assert_eq!(stats.total_tokens, 500); // 150 + 280 + 70
    assert_eq!(stats.total_cache_read_tokens, 90);
    assert_eq!(stats.total_cache_write_tokens, 30);
    assert_eq!(stats.request_count, 3);
}

#[test]
fn test_session_cache_hit_rate() {
    let mut session =
        ConversationSession::new("sess_stats_rate".into(), "gpt-4o".into(), tmp_path());
    // 100 prompt, 30 cache_read → rate = 0.3
    session.accumulate_usage(&make_usage(100, 50, Some(150), Some(30), None));
    let rate = session.stats().cache_hit_rate();
    assert!((rate - 0.3).abs() < f64::EPSILON);

    // Add 200 more prompt, 70 more cache_read → 100/300 ≈ 0.333...
    session.accumulate_usage(&make_usage(200, 50, Some(250), Some(70), None));
    let rate = session.stats().cache_hit_rate();
    assert!((rate - (100.0 / 300.0)).abs() < f64::EPSILON);
}

#[test]
fn test_session_cache_hit_rate_zero_prompt() {
    let session = ConversationSession::new("sess_stats_zero".into(), "gpt-4o".into(), tmp_path());
    assert_eq!(session.stats().cache_hit_rate(), 0.0);
}

#[test]
fn test_session_accumulate_usage_with_all_none_cache() {
    let mut session =
        ConversationSession::new("sess_stats_no_cache".into(), "gpt-4o".into(), tmp_path());
    session.accumulate_usage(&make_usage(50, 25, Some(75), None, None));
    let stats = session.stats();
    assert_eq!(stats.total_cache_read_tokens, 0);
    assert_eq!(stats.total_cache_write_tokens, 0);
    assert_eq!(stats.cache_hit_rate(), 0.0);
    assert_eq!(stats.request_count, 1);
}

#[test]
fn test_session_accumulate_usage_total_none_computed() {
    let mut session =
        ConversationSession::new("sess_stats_total_none".into(), "gpt-4o".into(), tmp_path());
    session.accumulate_usage(&make_usage(40, 10, None, Some(5), None));
    assert_eq!(session.stats().total_tokens, 50);
}

#[test]
fn test_session_total_cache_saved() {
    let mut session =
        ConversationSession::new("sess_stats_saved".into(), "gpt-4o".into(), tmp_path());
    session.accumulate_usage(&make_usage(100, 50, Some(150), Some(42), Some(10)));
    assert_eq!(session.stats().total_cache_saved(), 42);
}

// ── record_prompt_fingerprint integration ─────────────────────────────────

#[test]
fn test_session_record_prompt_fingerprint_initial_no_changes() {
    let mut session = ConversationSession::new("sess_fp_init".into(), "gpt-4o".into(), tmp_path());
    let tools = vec!["tool_a".to_string()];

    // First call: no previous fingerprint → pending_changes is None
    session.record_prompt_fingerprint(Some("You are helpful"), Some(&tools), None);
    assert!(session.stats().pending_changes.is_none());
}

#[test]
fn test_session_record_prompt_fingerprint_detects_system_prompt_change() {
    let mut session = ConversationSession::new("sess_fp_sys".into(), "gpt-4o".into(), tmp_path());
    let tools = vec!["tool_a".to_string()];

    session.record_prompt_fingerprint(Some("old prompt"), Some(&tools), None);
    session.record_prompt_fingerprint(Some("new prompt"), Some(&tools), None);

    let pc = session.stats().pending_changes.as_ref().unwrap();
    assert!(pc.system_prompt_changed);
    assert!(!pc.tools_changed);
}

#[test]
fn test_session_record_prompt_fingerprint_detects_tools_change() {
    let mut session = ConversationSession::new("sess_fp_tools".into(), "gpt-4o".into(), tmp_path());
    let tools_v1 = vec!["tool_a".to_string()];
    let tools_v2 = vec!["tool_a".to_string(), "tool_b".to_string()];

    session.record_prompt_fingerprint(Some("prompt"), Some(&tools_v1), None);
    session.record_prompt_fingerprint(Some("prompt"), Some(&tools_v2), None);

    let pc = session.stats().pending_changes.as_ref().unwrap();
    assert!(pc.tools_changed);
    assert!(!pc.system_prompt_changed);
}

#[test]
fn test_session_record_prompt_fingerprint_detects_headers_change() {
    let mut session = ConversationSession::new("sess_fp_hdr".into(), "gpt-4o".into(), tmp_path());
    let tools = vec!["tool_a".to_string()];
    let h1 = vec![("x-api-key", "abc")];
    let h2 = vec![("x-api-key", "xyz")];

    session.record_prompt_fingerprint(Some("prompt"), Some(&tools), Some(&h1));
    session.record_prompt_fingerprint(Some("prompt"), Some(&tools), Some(&h2));

    let pc = session.stats().pending_changes.as_ref().unwrap();
    assert!(pc.headers_changed);
    assert!(!pc.system_prompt_changed);
    assert!(!pc.tools_changed);
}
