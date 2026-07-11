//! Running statistics accumulator for cross-turn LLM usage tracking.
//!
//! `RunningStats` accumulates token usage across multiple API calls within
//! a session, including cache hit/write metrics, and exposes derived
//! statistics like cache hit rate.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Instant;

use crate::processor::UnifiedUsage;

// ── Pre-call fingerprint types ───────────────────────────────────

/// A fingerprint of the prompt components relevant to cache behavior.
///
/// Captured at pre-call time so that post-call can attribute cache
/// breaks to specific component changes.
///
/// `request_timestamp` is excluded from `PartialEq` (and therefore
/// `Eq`) because `Instant` does not implement equality traits.
#[derive(Debug, Clone)]
pub struct PromptFingerprint {
    /// Hash of the static portion of the system prompt.
    pub system_static_hash: Option<u64>,
    /// Hash of the sorted, joined tools list.
    pub tools_hash: Option<u64>,
    /// Hash of the normalized HTTP headers.
    pub headers_hash: Option<u64>,
    /// Wall-clock time when this fingerprint was recorded.
    pub request_timestamp: Option<Instant>,
}

impl PartialEq for PromptFingerprint {
    fn eq(&self, other: &Self) -> bool {
        self.system_static_hash == other.system_static_hash
            && self.tools_hash == other.tools_hash
            && self.headers_hash == other.headers_hash
        // request_timestamp is intentionally excluded
    }
}

impl PromptFingerprint {
    /// Computes a fingerprint from the given prompt components.
    pub fn compute(
        system_static: Option<&str>,
        tools: Option<&[String]>,
        headers: Option<&[(&str, &str)]>,
    ) -> Self {
        Self {
            system_static_hash: system_static.map(hash_str),
            tools_hash: tools.map(hash_tools),
            headers_hash: headers.map(hash_headers),
            request_timestamp: Some(Instant::now()),
        }
    }
}

/// Describes which prompt components changed between two consecutive
/// LLM calls (pre-call comparison).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingChanges {
    /// System prompt content changed since the last fingerprint.
    pub system_prompt_changed: bool,
    /// Tools list changed since the last fingerprint.
    pub tools_changed: bool,
    /// HTTP headers changed since the last fingerprint.
    pub headers_changed: bool,
    /// Wall-clock duration since the last fingerprint was recorded.
    pub time_since_last: Option<std::time::Duration>,
}

/// Possible root causes for a cache break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheBreakCause {
    /// The system prompt was rebuilt or modified.
    SystemPromptChanged,
    /// The tools list was modified.
    ToolsChanged,
    /// HTTP headers were modified.
    HeadersChanged,
    /// The time between calls exceeded the cache TTL.
    TtlExpired,
    /// The session was resumed from a saved state.
    SessionResumed,
    /// No specific cause could be determined.
    Unknown,
}

// ── Hash helpers ─────────────────────────────────────────────────

fn hash_str(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

fn hash_tools(tools: &[String]) -> u64 {
    let mut sorted: Vec<&str> = tools.iter().map(String::as_str).collect();
    sorted.sort();
    let joined = sorted.join("\x00");
    hash_str(&joined)
}

fn hash_headers(headers: &[(&str, &str)]) -> u64 {
    let mut sorted: Vec<(&str, &str)> = headers.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(b.0).then(a.1.cmp(b.1)));
    let joined: String = sorted
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("\x00");
    hash_str(&joined)
}

/// Default cache TTL in seconds.
///
/// If the time between consecutive LLM calls exceeds this value,
/// the cache break is attributed to TTL expiry.
const DEFAULT_CACHE_TTL_SECS: u64 = 300;

/// Compares two fingerprints and returns the detected pending changes.
fn compute_pending(prev: &PromptFingerprint, new: &PromptFingerprint) -> PendingChanges {
    let time_since_last = prev
        .request_timestamp
        .and_then(|t| new.request_timestamp.map(|now| now.duration_since(t)));

    PendingChanges {
        system_prompt_changed: prev.system_static_hash != new.system_static_hash,
        tools_changed: prev.tools_hash != new.tools_hash,
        headers_changed: prev.headers_hash != new.headers_hash,
        time_since_last,
    }
}

// ── Cache break info ─────────────────────────────────────────────

/// Information about a detected cache break between two consecutive calls.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheBreakInfo {
    /// Previous call's `cache_read_tokens` value.
    pub previous_cache_read: u32,
    /// Current call's `cache_read_tokens` value.
    pub current_cache_read: u32,
    /// Absolute drop in cache-read tokens.
    pub drop_tokens: u32,
    /// Ratio of the drop relative to the previous value (0.0–1.0).
    pub drop_ratio: f64,
    /// Attributed causes for this cache break.
    pub causes: Vec<CacheBreakCause>,
}

/// Detects a cache break between two consecutive cache-read token counts.
///
/// Returns `Some(CacheBreakInfo)` when:
/// - `current` is less than `previous` by more than 5%
///   (`current < previous * 0.95`) **and** the absolute drop exceeds
///   2 000 tokens.
///
/// Returns `None` when either input is `None`, the current value is
/// greater than or equal to the previous value, or the drop does not
/// meet the thresholds.
pub fn detect_cache_break(previous: Option<u32>, current: Option<u32>) -> Option<CacheBreakInfo> {
    let prev = previous?;
    let curr = current?;

    if curr >= prev {
        return None;
    }

    let drop_tokens = prev - curr;
    let threshold_tokens = 2000u32;

    if drop_tokens <= threshold_tokens {
        return None;
    }

    let drop_ratio = drop_tokens as f64 / prev as f64;
    if drop_ratio <= 0.05 {
        return None;
    }

    Some(CacheBreakInfo {
        previous_cache_read: prev,
        current_cache_read: curr,
        drop_tokens,
        drop_ratio,
        causes: vec![],
    })
}

/// Accumulated token usage statistics across multiple LLM API calls.
///
/// All fields use `u64` to avoid overflow in long sessions that may
/// exceed 4 billion tokens.
///
/// `PartialEq` compares all fields **except** `last_fingerprint`
/// (which contains `Instant`, not `Eq`).
#[derive(Debug, Clone)]
pub struct RunningStats {
    /// Cumulative prompt tokens across all calls.
    pub total_prompt_tokens: u64,
    /// Cumulative completion tokens across all calls.
    pub total_completion_tokens: u64,
    /// Cumulative total tokens across all calls.
    pub total_tokens: u64,
    /// Cumulative cache-read (hit) tokens.
    pub total_cache_read_tokens: u64,
    /// Cumulative cache-write (creation) tokens.
    pub total_cache_write_tokens: u64,
    /// Number of API calls accumulated.
    pub request_count: u64,
    /// `cache_read_tokens` from the most recent API call.
    ///
    /// `None` before any calls have been accumulated.
    pub last_cache_read_tokens: Option<u32>,
    /// The most recent pre-call fingerprint, or `None` if none
    /// recorded yet.
    pub last_fingerprint: Option<PromptFingerprint>,
    /// Pending component changes detected by comparing the latest
    /// fingerprint against `last_fingerprint`.
    pub pending_changes: Option<PendingChanges>,
}

impl PartialEq for RunningStats {
    fn eq(&self, other: &Self) -> bool {
        self.total_prompt_tokens == other.total_prompt_tokens
            && self.total_completion_tokens == other.total_completion_tokens
            && self.total_tokens == other.total_tokens
            && self.total_cache_read_tokens == other.total_cache_read_tokens
            && self.total_cache_write_tokens == other.total_cache_write_tokens
            && self.request_count == other.request_count
            && self.last_cache_read_tokens == other.last_cache_read_tokens
            && self.pending_changes == other.pending_changes
        // last_fingerprint excluded: Instant does not implement Eq
    }
}

impl Eq for RunningStats {}

impl RunningStats {
    /// Creates a new `RunningStats` with all counters zeroed.
    pub fn new() -> Self {
        Self {
            total_prompt_tokens: 0,
            total_completion_tokens: 0,
            total_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_write_tokens: 0,
            request_count: 0,
            last_cache_read_tokens: None,
            last_fingerprint: None,
            pending_changes: None,
        }
    }

    /// Detects a cache break using the previous and current
    /// `cache_read_tokens` values, then updates the tracked last value.
    ///
    /// Call this **before** `accumulate()` so that `last_cache_read_tokens`
    /// still holds the previous call's value when the comparison is made.
    pub fn detect_cache_break_and_update(
        &mut self,
        current_cache_read: Option<u32>,
    ) -> Option<CacheBreakInfo> {
        let mut info = detect_cache_break(self.last_cache_read_tokens, current_cache_read);
        self.last_cache_read_tokens = current_cache_read;

        // Attribute causes when a cache break is detected.
        if let Some(ref mut break_info) = info {
            break_info.causes = self.attribute_cache_break_causes();
            tracing::warn!(
                previous = break_info.previous_cache_read,
                current = break_info.current_cache_read,
                drop_tokens = break_info.drop_tokens,
                drop_ratio = break_info.drop_ratio,
                causes = ?break_info.causes,
                "KV cache break: prefix invalidated between consecutive calls"
            );
        }

        info
    }

    /// Derives cache break causes from `pending_changes` recorded
    /// during the pre-call fingerprint phase.
    ///
    /// Attribution rules:
    /// - `system_prompt_changed` → `SystemPromptChanged`
    /// - `tools_changed` → `ToolsChanged`
    /// - `headers_changed` → `HeadersChanged`
    /// - `time_since_last` > `DEFAULT_CACHE_TTL_SECS` → `TtlExpired`
    /// - `request_count == 0` or `last_cache_read_tokens` was
    ///   previously `None` → `SessionResumed`
    /// - No match → `Unknown`
    fn attribute_cache_break_causes(&mut self) -> Vec<CacheBreakCause> {
        let mut causes = Vec::new();

        if let Some(ref pc) = self.pending_changes {
            if pc.system_prompt_changed {
                causes.push(CacheBreakCause::SystemPromptChanged);
            }
            if pc.tools_changed {
                causes.push(CacheBreakCause::ToolsChanged);
            }
            if pc.headers_changed {
                causes.push(CacheBreakCause::HeadersChanged);
            }
            if let Some(dur) = pc.time_since_last {
                if dur.as_secs() > DEFAULT_CACHE_TTL_SECS {
                    causes.push(CacheBreakCause::TtlExpired);
                }
            }
        }

        // Session resumed: first cache_read with no prior value.
        // `request_count == 0` covers the case where `last_cache_read_tokens`
        // was already set but no previous accumulate occurred.
        if self.request_count == 0 && self.last_cache_read_tokens.is_some() {
            causes.push(CacheBreakCause::SessionResumed);
        }

        if causes.is_empty() {
            causes.push(CacheBreakCause::Unknown);
        }

        causes
    }

    /// Accumulates a single API call's usage into the running totals.
    ///
    /// `Option<u32>` fields that are `None` are treated as 0.
    /// When `total_tokens` is `None`, it is computed as
    /// `prompt_tokens + completion_tokens`.
    pub fn accumulate(&mut self, usage: &UnifiedUsage) {
        let prompt = u64::from(usage.prompt_tokens);
        let completion = u64::from(usage.completion_tokens);
        let total = usage
            .total_tokens
            .map(u64::from)
            .unwrap_or(prompt + completion);
        let cache_read = usage.cache_read_tokens.map_or(0u64, u64::from);
        let cache_write = usage.cache_write_tokens.map_or(0u64, u64::from);

        self.total_prompt_tokens += prompt;
        self.total_completion_tokens += completion;
        self.total_tokens += total;
        self.total_cache_read_tokens += cache_read;
        self.total_cache_write_tokens += cache_write;
        self.request_count += 1;
    }

    /// Returns the cache hit rate as a fraction in `[0.0, 1.0]`.
    ///
    /// Computed as `total_cache_read_tokens / total_prompt_tokens`.
    /// Returns `0.0` when `total_prompt_tokens` is zero to avoid
    /// division by zero.
    pub fn cache_hit_rate(&self) -> f64 {
        if self.total_prompt_tokens == 0 {
            return 0.0;
        }
        self.total_cache_read_tokens as f64 / self.total_prompt_tokens as f64
    }

    /// Returns the total number of tokens saved by cache hits.
    ///
    /// This is an alias for `total_cache_read_tokens`, provided
    /// for readability at call sites.
    pub fn total_cache_saved(&self) -> u64 {
        self.total_cache_read_tokens
    }

    /// Returns the `cache_read_tokens` from the most recent API call,
    /// or `None` if no calls have been accumulated yet.
    pub fn last_cache_read_tokens(&self) -> Option<u32> {
        self.last_cache_read_tokens
    }

    /// Records a pre-call fingerprint of prompt components and
    /// computes the changes since the last fingerprint.
    ///
    /// The resulting `PendingChanges` are stored in
    /// `self.pending_changes` and can be retrieved (and cleared)
    /// via [`take_pending_changes`](Self::take_pending_changes).
    pub fn record_fingerprint(
        &mut self,
        system_static: Option<&str>,
        tools: Option<&[String]>,
        headers: Option<&[(&str, &str)]>,
    ) {
        let new_fp = PromptFingerprint::compute(system_static, tools, headers);
        let pending = self
            .last_fingerprint
            .as_ref()
            .map(|prev| compute_pending(prev, &new_fp));
        self.pending_changes = pending;
        self.last_fingerprint = Some(new_fp);
    }

    /// Takes the pending changes (clearing the stored value).
    ///
    /// Returns `None` if [`record_fingerprint`](Self::record_fingerprint)
    /// has not yet been called, or if the pending changes were
    /// already consumed.
    pub fn take_pending_changes(&mut self) -> Option<PendingChanges> {
        self.pending_changes.take()
    }
}

impl Default for RunningStats {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_new_is_zeroed() {
        let stats = RunningStats::new();
        assert_eq!(stats.total_prompt_tokens, 0);
        assert_eq!(stats.total_completion_tokens, 0);
        assert_eq!(stats.total_tokens, 0);
        assert_eq!(stats.total_cache_read_tokens, 0);
        assert_eq!(stats.total_cache_write_tokens, 0);
        assert_eq!(stats.request_count, 0);
    }

    #[test]
    fn test_accumulate_basic() {
        let mut stats = RunningStats::new();
        stats.accumulate(&make_usage(100, 50, Some(150), Some(30), Some(20)));
        assert_eq!(stats.total_prompt_tokens, 100);
        assert_eq!(stats.total_completion_tokens, 50);
        assert_eq!(stats.total_tokens, 150);
        assert_eq!(stats.total_cache_read_tokens, 30);
        assert_eq!(stats.total_cache_write_tokens, 20);
        assert_eq!(stats.request_count, 1);

        stats.accumulate(&make_usage(200, 80, Some(280), Some(60), None));
        assert_eq!(stats.total_prompt_tokens, 300);
        assert_eq!(stats.total_completion_tokens, 130);
        assert_eq!(stats.total_tokens, 430);
        assert_eq!(stats.total_cache_read_tokens, 90);
        assert_eq!(stats.total_cache_write_tokens, 20);
        assert_eq!(stats.request_count, 2);
    }

    #[test]
    fn test_accumulate_all_none_cache_fields() {
        let mut stats = RunningStats::new();
        stats.accumulate(&make_usage(100, 50, Some(150), None, None));
        assert_eq!(stats.total_cache_read_tokens, 0);
        assert_eq!(stats.total_cache_write_tokens, 0);
    }

    #[test]
    fn test_accumulate_total_none_computed() {
        let mut stats = RunningStats::new();
        stats.accumulate(&make_usage(100, 50, None, None, None));
        assert_eq!(stats.total_tokens, 150);
    }

    #[test]
    fn test_accumulate_partial_none() {
        let mut stats = RunningStats::new();
        stats.accumulate(&make_usage(100, 50, None, Some(40), None));
        assert_eq!(stats.total_tokens, 150);
        assert_eq!(stats.total_cache_read_tokens, 40);
        assert_eq!(stats.total_cache_write_tokens, 0);
    }

    #[test]
    fn test_cache_hit_rate_normal() {
        let mut stats = RunningStats::new();
        stats.accumulate(&make_usage(100, 50, Some(150), Some(30), None));
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
        stats.accumulate(&make_usage(100, 50, Some(150), Some(42), Some(10)));
        assert_eq!(stats.total_cache_saved(), 42);
    }

    #[test]
    fn test_default_trait() {
        let stats = RunningStats::default();
        assert_eq!(stats.request_count, 0);
    }

    // ── detect_cache_break unit tests ──────────────────────────────

    #[test]
    fn detect_cache_break_returns_none_when_both_none() {
        assert!(detect_cache_break(None, None).is_none());
    }

    #[test]
    fn detect_cache_break_returns_none_when_prev_none() {
        assert!(detect_cache_break(None, Some(10000)).is_none());
    }

    #[test]
    fn detect_cache_break_returns_none_when_curr_none() {
        assert!(detect_cache_break(Some(10000), None).is_none());
    }

    #[test]
    fn detect_cache_break_returns_none_when_curr_equals_prev() {
        assert!(detect_cache_break(Some(10000), Some(10000)).is_none());
    }

    #[test]
    fn detect_cache_break_returns_none_when_curr_greater_than_prev() {
        assert!(detect_cache_break(Some(8000), Some(10000)).is_none());
    }

    #[test]
    fn detect_cache_break_returns_none_when_drop_exactly_2000() {
        assert!(detect_cache_break(Some(10000), Some(8000)).is_none());
    }

    #[test]
    fn detect_cache_break_returns_none_when_drop_below_2000() {
        assert!(detect_cache_break(Some(10000), Some(8500)).is_none());
    }

    #[test]
    fn detect_cache_break_returns_none_when_ratio_le_5_percent() {
        assert!(detect_cache_break(Some(100000), Some(95500)).is_none());
    }

    #[test]
    fn detect_cache_break_returns_none_when_ratio_exactly_5_percent() {
        assert!(detect_cache_break(Some(100000), Some(95000)).is_none());
    }

    #[test]
    fn detect_cache_break_returns_some_when_both_thresholds_met() {
        let info = detect_cache_break(Some(100000), Some(90000)).unwrap();
        assert_eq!(info.previous_cache_read, 100000);
        assert_eq!(info.current_cache_read, 90000);
        assert_eq!(info.drop_tokens, 10000);
        assert!((info.drop_ratio - 0.10).abs() < 1e-10);
    }

    #[test]
    fn detect_cache_break_large_drop() {
        let info = detect_cache_break(Some(50000), Some(30000)).unwrap();
        assert_eq!(info.drop_tokens, 20000);
        assert!((info.drop_ratio - 0.40).abs() < 1e-10);
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

    // ── RunningStats.last_cache_read_tokens integration tests ───────

    #[test]
    fn last_cache_read_tokens_none_before_any_accumulate() {
        let stats = RunningStats::new();
        assert_eq!(stats.last_cache_read_tokens, None);
    }

    #[test]
    fn last_cache_read_tokens_set_by_detect_cache_break_and_update() {
        let mut stats = RunningStats::new();
        stats.detect_cache_break_and_update(Some(3000));
        assert_eq!(stats.last_cache_read_tokens, Some(3000));
    }

    #[test]
    fn last_cache_read_tokens_tracks_latest_value_via_detect() {
        let mut stats = RunningStats::new();
        stats.detect_cache_break_and_update(Some(3000));
        assert_eq!(stats.last_cache_read_tokens, Some(3000));

        stats.detect_cache_break_and_update(Some(5000));
        assert_eq!(stats.last_cache_read_tokens, Some(5000));

        stats.detect_cache_break_and_update(Some(2000));
        assert_eq!(stats.last_cache_read_tokens, Some(2000));
    }

    #[test]
    fn last_cache_read_tokens_none_when_cache_read_none() {
        let mut stats = RunningStats::new();
        stats.accumulate(&make_usage(100, 50, Some(150), None, None));
        assert_eq!(stats.last_cache_read_tokens, None);
    }

    #[test]
    fn detect_cache_break_and_update_returns_none_first_call() {
        let mut stats = RunningStats::new();
        let result = stats.detect_cache_break_and_update(Some(10000));
        assert!(result.is_none());
        assert_eq!(stats.last_cache_read_tokens, Some(10000));
    }

    #[test]
    fn detect_cache_break_and_update_returns_none_when_no_break() {
        let mut stats = RunningStats::new();
        stats.detect_cache_break_and_update(Some(10000));
        let result = stats.detect_cache_break_and_update(Some(9900));
        assert!(result.is_none());
        assert_eq!(stats.last_cache_read_tokens, Some(9900));
    }

    #[test]
    fn detect_cache_break_and_update_returns_some_on_break() {
        let mut stats = RunningStats::new();
        stats.detect_cache_break_and_update(Some(100000));
        let result = stats.detect_cache_break_and_update(Some(90000));
        let info = result.unwrap();
        assert_eq!(info.previous_cache_read, 100000);
        assert_eq!(info.current_cache_read, 90000);
        assert_eq!(info.drop_tokens, 10000);
        assert_eq!(stats.last_cache_read_tokens, Some(90000));
    }

    #[test]
    fn detect_cache_break_and_update_chain() {
        let mut stats = RunningStats::new();
        stats.detect_cache_break_and_update(Some(50000));
        assert_eq!(stats.last_cache_read_tokens, Some(50000));

        let r1 = stats.detect_cache_break_and_update(Some(49000));
        assert!(r1.is_none());
        assert_eq!(stats.last_cache_read_tokens, Some(49000));

        let r2 = stats.detect_cache_break_and_update(Some(45000));
        let info = r2.unwrap();
        assert_eq!(info.previous_cache_read, 49000);
        assert_eq!(info.current_cache_read, 45000);
        assert_eq!(info.drop_tokens, 4000);
        assert_eq!(stats.last_cache_read_tokens, Some(45000));
    }
}
