//! Running statistics accumulator for cross-turn LLM usage tracking.
//!
//! `RunningStats` accumulates token usage across multiple API calls within a session,
//! including cache hit/write metrics, and exposes derived statistics like cache hit rate.

use crate::processor::UnifiedUsage;

// ── Cache anomaly tolerance ──────────────────────────────────────

/// Fixed tolerance (in tokens) for cache hit consistency checks.
///
/// Design-fixed value absorbing token-boundary differences and normal
/// fluctuation of cache hit counts. Not runtime-configurable; changing
/// it requires a design change.
const CACHE_TOLERANCE_TOKENS: u32 = 1024;

// ── Cache anomaly info ───────────────────────────────────────────

/// Information about a detected cache hit anomaly.
///
/// Carries the previous call's total prompt tokens (reference value),
/// the current call's cache-read tokens, their absolute deviation, and
/// the tolerance margin applied for this decision.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheAnomalyInfo {
    /// Previous call's total prompt tokens (reference value).
    pub previous_prompt_tokens: u32,
    /// Current call's cache-read (hit) tokens.
    pub current_cache_read_tokens: u32,
    /// Absolute deviation between hit count and reference value.
    pub deviation_tokens: u32,
    /// Tolerance margin applied for this decision.
    pub tolerance_tokens: u32,
}

/// Static text appended to cache-anomaly notifications listing common causes.
const POSSIBLE_CAUSES: &str = "可能原因：上下文前缀变更、缓存 TTL 过期、模型/参数变更";

impl CacheAnomalyInfo {
    /// Formats a user-facing notification for this cache anomaly.
    ///
    /// The notification includes the reference value, the current hit
    /// count, the deviation, the tolerance margin, followed by possible
    /// causes (static list, no runtime detection).
    pub fn format_notification(&self) -> String {
        format!(
            "[缓存命中异常] 参考值 {} tokens，本次命中 {} tokens，偏差 {} tokens（容忍余量 {} tokens）。{POSSIBLE_CAUSES}",
            self.previous_prompt_tokens,
            self.current_cache_read_tokens,
            self.deviation_tokens,
            self.tolerance_tokens,
        )
    }
}

/// Detects a cache hit anomaly for the current call.
///
/// The reference value is the previous call's total prompt tokens;
/// it is compared against the current call's cache-read tokens. Returns
/// `Some(CacheAnomalyInfo)` when the absolute deviation exceeds the
/// fixed tolerance ([`CACHE_TOLERANCE_TOKENS`]) — direction-agnostic,
/// so a hit count exceeding the reference is also an anomaly.
///
/// Returns `None` (no check / no anomaly) when:
/// - Either input is `None` (protocol did not provide the cache field).
///   `Some(0)` is a genuine zero hit and **does** participate.
/// - `previous_prompt_tokens` is `Some(0)`: upstream prompt counts are
///   synthesized to 0 when unavailable, so 0 marks the reference value
///   as unusable.
/// - The deviation is within the tolerance margin.
pub fn detect_cache_anomaly(
    previous_prompt_tokens: Option<u32>,
    current_cache_read_tokens: Option<u32>,
) -> Option<CacheAnomalyInfo> {
    let prev = previous_prompt_tokens?;
    let curr = current_cache_read_tokens?;
    if prev == 0 {
        return None;
    }
    let deviation = prev.abs_diff(curr);
    if deviation <= CACHE_TOLERANCE_TOKENS {
        return None;
    }
    Some(CacheAnomalyInfo {
        previous_prompt_tokens: prev,
        current_cache_read_tokens: curr,
        deviation_tokens: deviation,
        tolerance_tokens: CACHE_TOLERANCE_TOKENS,
    })
}

/// Current Unix epoch time in milliseconds.
fn current_epoch_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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
    /// Total prompt tokens of the most recent call; serves as the
    /// reference value for the next round's cache consistency check.
    ///
    /// `None` before any calls have been accumulated.
    pub last_prompt_tokens: Option<u32>,
    /// Completion tokens of the most recent call, for context-size
    /// estimation.
    pub last_completion_tokens: Option<u32>,
    /// Millisecond timestamp of the most recent call, for cache
    /// natural-expiry exemption checks.
    pub last_call_at: Option<u64>,
    /// Model/provider identifier of the most recent call, for
    /// model-switch exemption checks.
    pub last_call_model: Option<String>,
    /// `cache_read_tokens` from the most recent API call.
    ///
    /// `None` before any calls have been accumulated.
    pub last_cache_read_tokens: Option<u32>,
    /// Per-call cache hit rate from the previous API call.
    ///
    /// Computed as `cache_read / prompt_tokens` for that call.
    /// `None` before any calls have been accumulated.
    pub last_cache_hit_rate: Option<f64>,
    /// The most recent cache anomaly event, if any.
    pub last_cache_anomaly: Option<CacheAnomalyInfo>,
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
            && self.last_prompt_tokens == other.last_prompt_tokens
            && self.last_completion_tokens == other.last_completion_tokens
            && self.last_call_at == other.last_call_at
            && self.last_call_model == other.last_call_model
            && self.last_cache_read_tokens == other.last_cache_read_tokens
            && self.last_cache_hit_rate == other.last_cache_hit_rate
            && self.last_cache_anomaly == other.last_cache_anomaly
    }
}

impl Eq for RunningStats {}

/// Core accumulation and detection methods.
impl RunningStats {
    /// Creates a new `RunningStats` with all counters zeroed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Detects a cache hit anomaly using the previous call's state.
    ///
    /// Reference value is [`RunningStats::last_prompt_tokens`] (the
    /// previous call's total input tokens); compared against
    /// `current_cache_read`. The first call (no accumulated reference)
    /// is naturally exempt because `last_prompt_tokens` is `None`.
    ///
    /// Call this **before** [`RunningStats::accumulate`] so the
    /// reference value still holds the previous call's state.
    pub fn detect_cache_anomaly_and_update(
        &mut self,
        current_cache_read: Option<u32>,
        current_prompt_tokens: Option<u32>,
    ) -> Option<CacheAnomalyInfo> {
        let info = detect_cache_anomaly(self.last_prompt_tokens, current_cache_read);

        // Per-call snapshot fields (doc-silent, existing semantics kept):
        // providers that always return 0 for cache_read are not using
        // caching at all — treat as None so the snapshot excludes them.
        let effective_cache_read = current_cache_read.filter(|&v| v != 0);
        self.last_cache_read_tokens = effective_cache_read;

        // Compute current per-call hit rate.
        let current_rate = match (effective_cache_read, current_prompt_tokens) {
            (Some(cr), Some(pt)) if pt > 0 => Some(cr as f64 / pt as f64),
            _ => None,
        };
        self.last_cache_hit_rate = current_rate;

        // Store cache anomaly info when an anomaly is detected.
        if let Some(ref anomaly) = info {
            self.last_cache_anomaly = Some(anomaly.clone());
            Self::log_cache_anomaly(anomaly);
        }

        info
    }

    /// Logs a detected cache anomaly event.
    fn log_cache_anomaly(info: &CacheAnomalyInfo) {
        tracing::warn!(
            previous_prompt_tokens = info.previous_prompt_tokens,
            current_cache_read_tokens = info.current_cache_read_tokens,
            deviation_tokens = info.deviation_tokens,
            tolerance_tokens = info.tolerance_tokens,
            "Cache anomaly: cache hit deviates from previous prompt tokens beyond tolerance"
        );
    }

    /// Accumulates a single API call's usage into the running totals.
    ///
    /// `Option<u32>` fields that are `None` are treated as 0.
    /// When `total_tokens` is `None`, it is computed as
    /// `prompt_tokens + completion_tokens`.
    ///
    /// `model` is the model/provider identifier used for this call; it
    /// advances `last_call_model` for later model-switch exemption checks.
    pub fn accumulate(&mut self, usage: &UnifiedUsage, model: Option<&str>) {
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
        self.last_prompt_tokens = Some(usage.prompt_tokens);
        self.last_completion_tokens = Some(usage.completion_tokens);
        self.last_call_at = Some(current_epoch_millis());
        self.last_call_model = model.map(str::to_string);
    }
}

/// Helper getters and derived-statistics methods.
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

    /// Returns a reference to the most recent cache anomaly event, if any.
    pub fn last_cache_anomaly(&self) -> Option<&CacheAnomalyInfo> {
        self.last_cache_anomaly.as_ref()
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
        self.last_prompt_tokens = None;
        self.last_completion_tokens = None;
        self.last_call_at = None;
        self.last_call_model = None;
        self.last_cache_read_tokens = None;
        self.last_cache_hit_rate = None;
        self.last_cache_anomaly = None;
    }
}
