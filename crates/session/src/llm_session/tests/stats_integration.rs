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
