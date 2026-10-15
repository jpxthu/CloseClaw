//! Unit tests for the cache prefix snapshot and eligibility logic.
//!
//! Covers `build_prefix_snapshot`, `prefix_matches` (tail-append
//! extension semantics and mismatch boundaries) and
//! `cache_check_eligibility` (first-round / prefix / model-switch /
//! natural-expiry exemptions), per llm-session-enhancements.md
//! "缓存命中一致性校验" (:67-87).

use super::{
    build_prefix_snapshot, cache_check_eligibility, prefix_matches, CacheCheckEligibility,
    CacheExemptReason, PreviousCallState, RequestPrefixSnapshot,
};
use crate::llm_types::{InternalMessage, ToolDefinition};

const TTL_MS: u64 = 300_000;

// ── Test helpers ─────────────────────────────────────────────────

fn msg(role: &str, content: &str) -> InternalMessage {
    InternalMessage {
        role: role.to_string(),
        content: content.to_string(),
        ..Default::default()
    }
}

fn snapshot(system: &str, messages: &[InternalMessage]) -> RequestPrefixSnapshot {
    build_prefix_snapshot(Some(system), None, None, messages)
}

fn tool(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: format!("tool {name}"),
        input_schema: Some(serde_json::json!({ "type": "object" })),
        cache: true,
    }
}

fn prev_state<'a>(
    snapshot: Option<&'a RequestPrefixSnapshot>,
    model: Option<&'a str>,
    called_at_ms: Option<u64>,
) -> PreviousCallState<'a> {
    PreviousCallState {
        snapshot,
        model,
        called_at_ms,
    }
}

// ── prefix_matches: extension semantics ──────────────────────────

#[test]
fn prefix_matches_on_tail_append_only() {
    let base = [msg("user", "hi"), msg("assistant", "hello")];
    let prev = snapshot("sys", &base);
    let extended: Vec<_> = base
        .iter()
        .cloned()
        .chain([msg("user", "next question")])
        .collect();
    let cur = snapshot("sys", &extended);
    assert!(prefix_matches(&prev, &cur));
}

#[test]
fn prefix_matches_when_identical_equal_length() {
    let msgs = [msg("user", "hi"), msg("assistant", "hello")];
    let prev = snapshot("sys", &msgs);
    let cur = snapshot("sys", &msgs);
    assert!(prefix_matches(&prev, &cur));
}

#[test]
fn prefix_mismatches_on_system_change() {
    let msgs = [msg("user", "hi")];
    let prev = snapshot("old system", &msgs);
    let cur = snapshot("new system", &msgs);
    assert!(!prefix_matches(&prev, &cur));
}

#[test]
fn prefix_mismatches_on_dynamic_system_change() {
    let msgs = [msg("user", "hi")];
    let prev = build_prefix_snapshot(Some("sys"), Some("dynamic v1"), None, &msgs);
    let cur = build_prefix_snapshot(Some("sys"), Some("dynamic v2"), None, &msgs);
    assert!(!prefix_matches(&prev, &cur));
}

#[test]
fn prefix_matches_when_only_dynamic_system_same() {
    let msgs = [msg("user", "hi")];
    let prev = build_prefix_snapshot(Some("sys"), Some("dynamic"), None, &msgs);
    let cur = build_prefix_snapshot(Some("sys"), Some("dynamic"), None, &msgs);
    assert!(prefix_matches(&prev, &cur));
}

#[test]
fn prefix_mismatches_on_middle_message_rewrite() {
    let prev_msgs = [msg("user", "q1"), msg("assistant", "a1"), msg("user", "q2")];
    let prev = snapshot("sys", &prev_msgs);
    let mut cur_msgs = prev_msgs.to_vec();
    cur_msgs.push(msg("assistant", "a2"));
    cur_msgs[1].content = "a1 (rewritten)".to_string();
    let cur = snapshot("sys", &cur_msgs);
    assert!(!prefix_matches(&prev, &cur));
}

#[test]
fn prefix_mismatches_on_same_length_different_content() {
    let prev = snapshot("sys", &[msg("user", "q1"), msg("assistant", "a1")]);
    let cur = snapshot("sys", &[msg("user", "q1"), msg("assistant", "different")]);
    assert!(!prefix_matches(&prev, &cur));
}

#[test]
fn prefix_mismatches_when_current_shorter_than_previous() {
    let prev = snapshot("sys", &[msg("user", "q1"), msg("assistant", "a1")]);
    let cur = snapshot("sys", &[msg("user", "q1")]);
    assert!(!prefix_matches(&prev, &cur));
}

// ── prefix_matches: tools representation ─────────────────────────

#[test]
fn prefix_mismatches_on_tools_change() {
    let msgs = [msg("user", "hi")];
    let prev = build_prefix_snapshot(Some("sys"), None, Some(&[tool("search")]), &msgs);
    let cur = build_prefix_snapshot(Some("sys"), None, Some(&[tool("fetch")]), &msgs);
    assert!(!prefix_matches(&prev, &cur));
}

#[test]
fn prefix_matches_with_unchanged_tools() {
    let msgs = [msg("user", "hi")];
    let prev = build_prefix_snapshot(Some("sys"), None, Some(&[tool("search")]), &msgs);
    let cur = build_prefix_snapshot(Some("sys"), None, Some(&[tool("search")]), &msgs);
    assert!(prefix_matches(&prev, &cur));
}

#[test]
fn prefix_matches_when_only_cache_marking_flag_differs() {
    // The per-round cache marking flag is not definition content.
    let msgs = [msg("user", "hi")];
    let mut unmarked = tool("search");
    unmarked.cache = false;
    let prev = build_prefix_snapshot(Some("sys"), None, Some(&[tool("search")]), &msgs);
    let cur = build_prefix_snapshot(Some("sys"), None, Some(&[unmarked]), &msgs);
    assert!(prefix_matches(&prev, &cur));
}

#[test]
fn prefix_matches_between_no_tools_and_empty_tools() {
    let msgs = [msg("user", "hi")];
    let prev = build_prefix_snapshot(Some("sys"), None, None, &msgs);
    let cur = build_prefix_snapshot(Some("sys"), None, Some(&[]), &msgs);
    assert!(prefix_matches(&prev, &cur));
}

// ── cache_check_eligibility: exemptions ──────────────────────────

#[test]
fn first_round_without_snapshot_is_exempt_as_new_baseline() {
    let cur = snapshot("sys", &[msg("user", "hi")]);
    let state = prev_state(None, Some("provider/model"), Some(1_000));
    assert_eq!(
        cache_check_eligibility(&state, &cur, Some("provider/model"), 2_000, TTL_MS),
        CacheCheckEligibility::Exempt(CacheExemptReason::NoPreviousSnapshot)
    );
}

#[test]
fn prefix_mismatch_is_exempt() {
    let msgs = [msg("user", "hi")];
    let prev = snapshot("sys", &msgs);
    let cur = snapshot("changed sys", &msgs);
    let state = prev_state(Some(&prev), Some("provider/model"), Some(1_000));
    assert_eq!(
        cache_check_eligibility(&state, &cur, Some("provider/model"), 2_000, TTL_MS),
        CacheCheckEligibility::Exempt(CacheExemptReason::PrefixMismatch)
    );
}

#[test]
fn model_switch_is_exempt() {
    let cur = snapshot("sys", &[msg("user", "hi")]);
    let state = prev_state(Some(&cur), Some("provider/old-model"), Some(1_000));
    assert_eq!(
        cache_check_eligibility(&state, &cur, Some("provider/new-model"), 2_000, TTL_MS),
        CacheCheckEligibility::Exempt(CacheExemptReason::ModelSwitch)
    );
}

#[test]
fn interval_exceeding_ttl_is_exempt_as_natural_expiry() {
    let cur = snapshot("sys", &[msg("user", "hi")]);
    let state = prev_state(Some(&cur), Some("provider/model"), Some(1_000));
    assert_eq!(
        cache_check_eligibility(
            &state,
            &cur,
            Some("provider/model"),
            1_000 + TTL_MS + 1,
            TTL_MS
        ),
        CacheCheckEligibility::Exempt(CacheExemptReason::CacheExpired)
    );
}

#[test]
fn unknown_previous_timestamp_is_exempt_as_expired() {
    let cur = snapshot("sys", &[msg("user", "hi")]);
    let state = prev_state(Some(&cur), Some("provider/model"), None);
    assert_eq!(
        cache_check_eligibility(&state, &cur, Some("provider/model"), 2_000, TTL_MS),
        CacheCheckEligibility::Exempt(CacheExemptReason::CacheExpired)
    );
}

// ── cache_check_eligibility: participation & boundaries ──────────

#[test]
fn appended_round_with_same_model_within_ttl_participates() {
    let base = [msg("user", "hi")];
    let prev = snapshot("sys", &base);
    let extended: Vec<_> = base
        .iter()
        .cloned()
        .chain([msg("assistant", "hello")])
        .collect();
    let cur = snapshot("sys", &extended);
    let state = prev_state(Some(&prev), Some("provider/model"), Some(1_000));
    assert_eq!(
        cache_check_eligibility(&state, &cur, Some("provider/model"), 1_000 + TTL_MS, TTL_MS),
        CacheCheckEligibility::Participate
    );
}

#[test]
fn interval_exactly_ttl_still_participates() {
    // 超过有效期才豁免：等于 TTL 未超过，参与校验。
    let cur = snapshot("sys", &[msg("user", "hi")]);
    let state = prev_state(Some(&cur), Some("provider/model"), Some(1_000));
    assert_eq!(
        cache_check_eligibility(&state, &cur, Some("provider/model"), 1_000 + TTL_MS, TTL_MS),
        CacheCheckEligibility::Participate
    );
}

#[test]
fn model_identifier_absent_on_both_sides_participates() {
    // fake/stub 路径无实际标识，双方按会话配置口径退化后均为 None。
    let cur = snapshot("sys", &[msg("user", "hi")]);
    let state = prev_state(Some(&cur), None, Some(1_000));
    assert_eq!(
        cache_check_eligibility(&state, &cur, None, 2_000, TTL_MS),
        CacheCheckEligibility::Participate
    );
}

#[test]
fn model_present_vs_absent_is_treated_as_switch() {
    let cur = snapshot("sys", &[msg("user", "hi")]);
    let state = prev_state(Some(&cur), Some("provider/model"), Some(1_000));
    assert_eq!(
        cache_check_eligibility(&state, &cur, None, 2_000, TTL_MS),
        CacheCheckEligibility::Exempt(CacheExemptReason::ModelSwitch)
    );
}

#[test]
fn prefix_mismatch_wins_over_model_switch_and_ttl() {
    // 多项豁免同时成立时按固定顺序取首个原因（prefix 优先）。
    let msgs = [msg("user", "hi")];
    let prev = snapshot("sys", &msgs);
    let cur = snapshot("changed sys", &msgs);
    let state = prev_state(Some(&prev), Some("provider/old-model"), None);
    assert_eq!(
        cache_check_eligibility(&state, &cur, Some("provider/new-model"), 2_000, TTL_MS),
        CacheCheckEligibility::Exempt(CacheExemptReason::PrefixMismatch)
    );
}

// ── build_prefix_snapshot ────────────────────────────────────────

#[test]
fn snapshot_joins_static_and_dynamic_system_sections() {
    let msgs = [msg("user", "hi")];
    let joined = build_prefix_snapshot(Some("static part"), Some("dynamic part"), None, &msgs);
    let manual = snapshot("static part\ndynamic part", &msgs);
    assert_eq!(joined, manual);
}

#[test]
fn snapshot_without_system_hashes_empty_system() {
    let msgs = [msg("user", "hi")];
    let no_system = build_prefix_snapshot(None, None, None, &msgs);
    let empty_system = snapshot("", &msgs);
    assert_eq!(no_system, empty_system);
}

#[test]
fn snapshot_hash_differs_for_different_message_content() {
    let prev = snapshot("sys", &[msg("user", "q1")]);
    let cur = snapshot("sys", &[msg("user", "q2")]);
    assert_ne!(prev.message_hashes, cur.message_hashes);
    assert!(!prefix_matches(&prev, &cur));
}
