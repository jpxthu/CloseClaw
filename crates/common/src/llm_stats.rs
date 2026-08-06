//! Running statistics accumulator for cross-turn LLM usage tracking.
//!
//! `RunningStats` accumulates token usage across multiple API calls within a session,
//! including cache hit/write metrics, and exposes derived statistics like cache hit rate.

use crate::processor::UnifiedUsage;

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
}

/// Static text appended to cache-break notifications listing common causes.
const POSSIBLE_CAUSES: &str = "可能原因：上下文变更、缓存 TTL 过期、模型/参数变更";

impl CacheBreakInfo {
    /// Formats a user-facing notification for this cache break.
    ///
    /// The notification includes the hit-rate comparison and token drop,
    /// followed by possible causes (static list, no runtime detection).
    pub fn format_notification(&self) -> String {
        let drop_pct = self.drop_ratio * 100.0;
        let prev_rate_pct = self.previous_hit_rate * 100.0;
        let curr_rate_pct = self.current_hit_rate * 100.0;
        format!(
            "[缓存断点] 缓存命中率从 {:.1}% 降至 {:.1}%（减少 {} tokens，降幅 {:.1}%）。{POSSIBLE_CAUSES}",
            prev_rate_pct, curr_rate_pct, self.drop_tokens, drop_pct,
        )
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
    })
}

/// Accumulated token usage statistics across multiple LLM API calls.
///
/// All fields use `u64` to avoid overflow in long sessions that may
/// exceed 4 billion tokens.
#[derive(Debug, Clone, Default)]
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
    }
}

impl Eq for RunningStats {}

/// Core accumulation and detection methods.
impl RunningStats {
    /// Creates a new `RunningStats` with all counters zeroed.
    pub fn new() -> Self {
        Self::default()
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
        let mut info = detect_cache_break(
            self.last_cache_read_tokens,
            current_cache_read,
            self.cache_break_thresholds.as_ref(),
        );
        // Save previous value for rate-based comparison; update after rate check.
        let prev_cache_read = self.last_cache_read_tokens;
        self.last_cache_read_tokens = current_cache_read;

        // Compute current per-call hit rate.
        let current_rate = match (current_cache_read, current_prompt_tokens) {
            (Some(cr), Some(pt)) if pt > 0 => Some(cr as f64 / pt as f64),
            _ => None,
        };

        // Override with hit-rate based detection when both rates available.
        if let (Some(prev), Some(curr)) = (prev_rate, current_rate) {
            self.apply_rate_break(&mut info, prev, curr, prev_cache_read, current_cache_read);
            // If token break exists but rate drop didn't trigger, fill rates.
            if let Some(ref mut b) = info {
                if b.previous_hit_rate == 0.0 && b.current_hit_rate == 0.0 {
                    b.previous_hit_rate = prev;
                    b.current_hit_rate = curr;
                }
            }
        }

        self.last_cache_hit_rate = current_rate;

        // Store cache break info when a break is detected.
        if let Some(ref break_info) = info {
            self.last_cache_break = Some(break_info.clone());
            Self::log_cache_break(break_info);
        }

        info
    }

    /// Logs a detected cache break event.
    fn log_cache_break(break_info: &CacheBreakInfo) {
        tracing::warn!(
            previous = break_info.previous_cache_read,
            current = break_info.current_cache_read,
            drop_tokens = break_info.drop_tokens,
            drop_ratio = break_info.drop_ratio,
            previous_hit_rate = break_info.previous_hit_rate,
            current_hit_rate = break_info.current_hit_rate,
            "KV cache break: prefix invalidated between consecutive calls"
        );
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
}

/// Helper getters, setters, and derived-statistics methods.
impl RunningStats {
    /// Returns the cache hit rate as a fraction in `[0.0, 1.0]`.
    /// Returns `0.0` when `total_prompt_tokens` is zero.
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

    /// Resets all counters and snapshots to initial values.
    /// Intended for session-end cleanup.
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
        self.last_cache_break = None;
    }

    /// Checks whether a hit-rate drop between consecutive calls exceeds
    /// the configured threshold.
    ///
    /// Returns `true` when the rate drop exceeds `drop_ratio_threshold`.
    fn did_rate_drop_exceed_threshold(&self, prev_rate: f64, current_rate: f64) -> bool {
        let th = self.cache_break_thresholds.clone().unwrap_or_default();
        (prev_rate - current_rate) > th.drop_ratio_threshold
    }

    /// Applies rate-based break detection: either enhances an existing
    /// token break with hit-rate data, or creates a rate-only break.
    fn apply_rate_break(
        &self,
        info: &mut Option<CacheBreakInfo>,
        prev_rate: f64,
        current_rate: f64,
        prev_cache_read: Option<u32>,
        current_cache_read: Option<u32>,
    ) {
        if !self.did_rate_drop_exceed_threshold(prev_rate, current_rate) {
            return;
        }
        if let Some(ref mut b) = info {
            b.previous_hit_rate = prev_rate;
            b.current_hit_rate = current_rate;
        } else {
            let prev_ct = prev_cache_read.unwrap_or(0);
            let curr_ct = current_cache_read.unwrap_or(0);
            let drop_tokens = prev_ct.saturating_sub(curr_ct);
            let drop_ratio = if prev_ct > 0 {
                drop_tokens as f64 / prev_ct as f64
            } else {
                0.0
            };
            *info = Some(CacheBreakInfo {
                previous_cache_read: prev_ct,
                current_cache_read: curr_ct,
                drop_tokens,
                drop_ratio,
                previous_hit_rate: prev_rate,
                current_hit_rate: current_rate,
            });
        }
    }
}
