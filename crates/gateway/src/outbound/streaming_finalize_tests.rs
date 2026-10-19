//! Unit tests for the private `build_stream_result` constructor —
//! the shared `StreamResult` literal convergence for the
//! registry-missing early-return and normal pipeline paths of
//! `finish_streaming_pipeline` (issue #3806 Step 1.2).
//!
//! Test dimensions:
//! 1. Normal path: `usage_override = Some(x)` wins → `usage == x`
//! 2. Normal path: `content_blocks` / `dsl_result` pass through verbatim
//! 3. Fallback path: `usage_override = None` → `usage == state.usage`
//! 4. Fixed contract: `retry_attempts == 0` and `call_target == None`
//!    (call sites attach the actual target afterwards)

use super::*;

fn state_usage() -> UnifiedUsage {
    UnifiedUsage {
        prompt_tokens: 1,
        completion_tokens: 2,
        total_tokens: Some(3),
        reasoning_tokens: None,
        cache_read_tokens: Some(5),
        cache_write_tokens: None,
    }
}

/// Distinct from [`state_usage`] so override-vs-fallback is
/// distinguishable by value comparison.
fn override_usage() -> UnifiedUsage {
    UnifiedUsage {
        prompt_tokens: 11,
        completion_tokens: 22,
        total_tokens: Some(33),
        reasoning_tokens: Some(4),
        cache_read_tokens: None,
        cache_write_tokens: None,
    }
}

fn sample_blocks() -> Vec<ContentBlock> {
    vec![
        ContentBlock::Text("hello".to_string()),
        ContentBlock::Text(" world".to_string()),
    ]
}

fn state_with_usage(usage: UnifiedUsage) -> StreamState {
    let mut state = StreamState::new(VerbosityLevel::default());
    state.usage = usage;
    state
}

/// Normal path: `usage_override = Some(x)` wins over `state.usage`;
/// `content_blocks` / `dsl_result` pass through verbatim.
#[test]
fn test_build_stream_result_override_wins_and_fields_pass_through() {
    let state = state_with_usage(state_usage());
    let dsl = Some("{\"instructions\":[]}".to_string());
    let result = build_stream_result(sample_blocks(), dsl.clone(), Some(override_usage()), &state);
    assert_eq!(result.usage, override_usage());
    assert_eq!(result.content_blocks, sample_blocks());
    assert_eq!(result.dsl_result, dsl);
}

/// Fallback path: `usage_override = None` falls back to `state.usage`.
#[test]
fn test_build_stream_result_usage_falls_back_to_state() {
    let state = state_with_usage(state_usage());
    let result = build_stream_result(sample_blocks(), None, None, &state);
    assert_eq!(result.usage, state_usage());
    assert_eq!(result.content_blocks, sample_blocks());
    assert_eq!(result.dsl_result, None);
}

/// Fixed contract: `retry_attempts == 0` and `call_target == None` —
/// the production call site attaches the actual target from the
/// invocation's returned value afterwards.
#[test]
fn test_build_stream_result_fixed_contract_fields() {
    let state = state_with_usage(state_usage());
    let result = build_stream_result(sample_blocks(), None, Some(override_usage()), &state);
    assert_eq!(result.retry_attempts, 0);
    assert!(result.call_target.is_none());
}
