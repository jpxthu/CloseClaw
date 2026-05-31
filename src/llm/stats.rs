//! Running statistics accumulator for cross-turn LLM usage tracking.
//!
//! `RunningStats` accumulates token usage across multiple API calls within
//! a session, including cache hit/write metrics, and exposes derived
//! statistics like cache hit rate.

use super::types::UnifiedUsage;

/// Accumulated token usage statistics across multiple LLM API calls.
///
/// All fields use `u64` to avoid overflow in long sessions that may
/// exceed 4 billion tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

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
        }
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
}
