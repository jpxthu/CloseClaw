//! Running statistics accumulator for cross-turn LLM usage tracking.
//!
//! `RunningStats` accumulates token usage across multiple API calls within a session,
//! including cache hit/write metrics, and exposes derived statistics like cache hit rate.

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

// ── Cache break threshold configuration ───────────────────────

/// Thresholds for cache break detection.
///
/// `drop_ratio_threshold` is the minimum fraction (0.0–1.0) of
/// hit-rate decline that triggers a cache break.
/// `min_drop_tokens` is the minimum absolute token drop required
/// for the rate-based comparison to activate.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheBreakThresholds {
    /// Minimum hit-rate drop ratio to trigger a break.
    pub drop_ratio_threshold: f64,
    /// Minimum absolute token drop (per call) to consider a break.
    pub min_drop_tokens: u32,
}

impl Default for CacheBreakThresholds {
    fn default() -> Self {
        Self {
            drop_ratio_threshold: 0.05,
            min_drop_tokens: 2000,
        }
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
    /// Per-call cache hit rate of the previous call (0.0–1.0).
    pub previous_hit_rate: f64,
    /// Per-call cache hit rate of the current call (0.0–1.0).
    pub current_hit_rate: f64,
    /// Attributed causes for this cache break.
    pub causes: Vec<CacheBreakCause>,
}

impl CacheBreakCause {
    /// Returns `(description, dimension)` for this cause.
    ///
    /// `description` is a human-readable Chinese label;
    /// `dimension` is the English cache-dimension identifier
    /// matching the code fingerprint key (e.g. `"system prompt"`).
    fn label(&self) -> (&'static str, &'static str) {
        match self {
            Self::SystemPromptChanged => ("system prompt 变更", "system prompt"),
            Self::ToolsChanged => ("工具列表变更", "tools list"),
            Self::HeadersChanged => ("请求头变更", "headers"),
            Self::TtlExpired => ("缓存 TTL 过期", "cache ttl"),
            Self::SessionResumed => ("会话恢复", "session state"),
            Self::Unknown => ("未知原因", "unknown"),
        }
    }
}

impl CacheBreakInfo {
    /// Formats a user-facing notification for this cache break.
    ///
    /// The notification includes the hit-rate comparison and token drop,
    /// attributed causes (in Chinese), and affected cache dimensions.
    /// When `causes` is empty, the cause/dimension clauses are omitted.
    pub fn format_notification(&self) -> String {
        let drop_pct = self.drop_ratio * 100.0;
        let prev_rate_pct = self.previous_hit_rate * 100.0;
        let curr_rate_pct = self.current_hit_rate * 100.0;
        let mut text = format!(
            "[缓存断点] 缓存命中率从 {:.1}% 降至 {:.1}%（减少 {} tokens，降幅 {:.1}%）。",
            prev_rate_pct, curr_rate_pct, self.drop_tokens, drop_pct,
        );
        if !self.causes.is_empty() {
            let causes_str = self
                .causes
                .iter()
                .map(|c| c.label().0)
                .collect::<Vec<_>>()
                .join("、");
            let dimensions_str = self
                .causes
                .iter()
                .map(|c| c.label().1)
                .collect::<Vec<_>>()
                .join("、");
            text.push_str(&format!(
                "原因：{}。受影响维度：{}。",
                causes_str, dimensions_str,
            ));
        }
        text
    }
}

/// Detects a cache break between two consecutive cache-read token counts.
///
/// Returns `Some(CacheBreakInfo)` when:
/// - `current` is less than `previous` by more than `thresholds.drop_ratio_threshold`
///   **and** the absolute drop exceeds `thresholds.min_drop_tokens`.
///
/// Uses [`CacheBreakThresholds::default`] when `thresholds` is `None`.
///
/// Returns `None` when either input is `None`, the current value is
/// greater than or equal to the previous value, or the drop does not
/// meet the thresholds.
pub fn detect_cache_break(
    previous: Option<u32>,
    current: Option<u32>,
    thresholds: Option<&CacheBreakThresholds>,
) -> Option<CacheBreakInfo> {
    let prev = previous?;
    let curr = current?;
    let th = thresholds.cloned().unwrap_or_default();

    if curr >= prev {
        return None;
    }

    let drop_tokens = prev - curr;

    if drop_tokens <= th.min_drop_tokens {
        return None;
    }

    let drop_ratio = drop_tokens as f64 / prev as f64;
    if drop_ratio <= th.drop_ratio_threshold {
        return None;
    }

    Some(CacheBreakInfo {
        previous_cache_read: prev,
        current_cache_read: curr,
        drop_tokens,
        drop_ratio,
        previous_hit_rate: 0.0,
        current_hit_rate: 0.0,
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
    /// Cumulative reasoning tokens across all calls.
    pub total_reasoning_tokens: u64,
    /// Custom cache break detection thresholds.
    ///
    /// When `None`, [`CacheBreakThresholds::default`] is used.
    pub cache_break_thresholds: Option<CacheBreakThresholds>,
    /// `cache_read_tokens` from the most recent API call.
    ///
    /// `None` before any calls have been accumulated.
    pub last_cache_read_tokens: Option<u32>,
    /// Per-call cache hit rate from the previous API call.
    ///
    /// Computed as `cache_read / prompt_tokens` for that call.
    /// `None` before any calls have been accumulated.
    pub last_cache_hit_rate: Option<f64>,
    /// The most recent pre-call fingerprint, or `None` if none
    /// recorded yet.
    pub last_fingerprint: Option<PromptFingerprint>,
    /// Pending component changes detected by comparing the latest
    /// fingerprint against `last_fingerprint`.
    pub pending_changes: Option<PendingChanges>,
    /// The most recent cache break event, if any.
    pub last_cache_break: Option<CacheBreakInfo>,
}

impl PartialEq for RunningStats {
    fn eq(&self, other: &Self) -> bool {
        self.total_prompt_tokens == other.total_prompt_tokens
            && self.total_completion_tokens == other.total_completion_tokens
            && self.total_tokens == other.total_tokens
            && self.total_cache_read_tokens == other.total_cache_read_tokens
            && self.total_cache_write_tokens == other.total_cache_write_tokens
            && self.request_count == other.request_count
            && self.total_reasoning_tokens == other.total_reasoning_tokens
            && self.cache_break_thresholds == other.cache_break_thresholds
            && self.last_cache_read_tokens == other.last_cache_read_tokens
            && self.last_cache_hit_rate == other.last_cache_hit_rate
            && self.last_cache_break == other.last_cache_break
            && self.pending_changes == other.pending_changes
        // last_fingerprint excluded: Instant does not implement Eq
    }
}

impl Eq for RunningStats {}

/// Core accumulation and detection methods.
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
            total_reasoning_tokens: 0,
            cache_break_thresholds: None,
            last_cache_read_tokens: None,
            last_cache_hit_rate: None,
            last_fingerprint: None,
            pending_changes: None,
            last_cache_break: None,
        }
    }

    /// Detects a cache break by comparing per-call hit rates.
    ///
    /// Computes the current hit rate as `current_cache_read /
    /// current_prompt_tokens` and compares it against the previous
    /// call's hit rate (`last_cache_hit_rate`). If the drop exceeds
    /// the threshold and the absolute token drop is significant,
    /// returns `Some(CacheBreakInfo)`.
    ///
    /// Call this **before** `accumulate()` so that `last_cache_read_tokens`
    /// still holds the previous call's value when the comparison is made.
    pub fn detect_cache_break_and_update(
        &mut self,
        current_cache_read: Option<u32>,
        current_prompt_tokens: Option<u32>,
    ) -> Option<CacheBreakInfo> {
        let prev_rate = self.last_cache_hit_rate;
        let mut info = detect_cache_break(self.last_cache_read_tokens, current_cache_read, None);
        self.last_cache_read_tokens = current_cache_read;

        // Compute current per-call hit rate.
        let current_rate = match (current_cache_read, current_prompt_tokens) {
            (Some(cr), Some(pt)) if pt > 0 => Some(cr as f64 / pt as f64),
            _ => None,
        };

        // Override with hit-rate based detection when both rates available.
        if let (Some(prev), Some(curr)) = (prev_rate, current_rate) {
            let th = self.cache_break_thresholds.clone().unwrap_or_default();
            let rate_drop = prev - curr;
            if rate_drop > th.drop_ratio_threshold {
                let drop_ok = info
                    .as_ref()
                    .map(|i| i.drop_tokens > th.min_drop_tokens)
                    .unwrap_or(false);
                if drop_ok {
                    // Enhance existing info with hit-rate data.
                    if let Some(ref mut b) = info {
                        b.previous_hit_rate = prev;
                        b.current_hit_rate = curr;
                    }
                } else {
                    // Trigger break based on rate drop alone.
                    let prev_ct = self.last_cache_read_tokens.unwrap_or(0);
                    let curr_ct = current_cache_read.unwrap_or(0);
                    let drop_tokens = prev_ct.saturating_sub(curr_ct);
                    let drop_ratio = if prev_ct > 0 {
                        drop_tokens as f64 / prev_ct as f64
                    } else {
                        0.0
                    };
                    info = Some(CacheBreakInfo {
                        previous_cache_read: prev_ct,
                        current_cache_read: curr_ct,
                        drop_tokens,
                        drop_ratio,
                        previous_hit_rate: prev,
                        current_hit_rate: curr,
                        causes: vec![],
                    });
                }
            } else if let Some(ref mut b) = info {
                // Even if no break, fill in hit rates for diagnostics.
                b.previous_hit_rate = prev;
                b.current_hit_rate = curr;
            }
        }

        self.last_cache_hit_rate = current_rate;

        // Attribute causes when a cache break is detected.
        if let Some(ref mut break_info) = info {
            break_info.causes = self.attribute_cache_break_causes();
            self.last_cache_break = Some(break_info.clone());
            tracing::warn!(
                previous = break_info.previous_cache_read,
                current = break_info.current_cache_read,
                drop_tokens = break_info.drop_tokens,
                drop_ratio = break_info.drop_ratio,
                previous_hit_rate = break_info.previous_hit_rate,
                current_hit_rate = break_info.current_hit_rate,
                causes = ?break_info.causes,
                "KV cache break: prefix invalidated between consecutive calls"
            );
        }

        info
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
        self.total_reasoning_tokens += usage.reasoning_tokens.map_or(0u64, u64::from);
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

    /// Returns a reference to the most recent cache break event, if any.
    pub fn last_cache_break(&self) -> Option<&CacheBreakInfo> {
        self.last_cache_break.as_ref()
    }

    /// Sets custom cache break detection thresholds.
    pub fn set_cache_break_thresholds(&mut self, thresholds: CacheBreakThresholds) {
        self.cache_break_thresholds = Some(thresholds);
    }

    /// Resets all counters, snapshots, and fingerprint data to their
    /// initial values.
    ///
    /// This is intended to be called at session end to avoid stale
    /// statistics carrying over into a new session.
    pub fn reset(&mut self) {
        self.total_prompt_tokens = 0;
        self.total_completion_tokens = 0;
        self.total_tokens = 0;
        self.total_cache_read_tokens = 0;
        self.total_cache_write_tokens = 0;
        self.request_count = 0;
        self.total_reasoning_tokens = 0;
        self.cache_break_thresholds = None;
        self.last_cache_read_tokens = None;
        self.last_cache_hit_rate = None;
        self.last_fingerprint = None;
        self.pending_changes = None;
        self.last_cache_break = None;
    }
}

/// Fingerprint recording and cache-break attribution.
impl RunningStats {
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
    fn attribute_cache_break_causes(&self) -> Vec<CacheBreakCause> {
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
