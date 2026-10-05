//! Metrics emission defaults used by the daemon composition root.
//!
//! Provides [`NoopMetricsEmitter`] — a zero-cost no-op backend for
//! [`MetricsEmitter`](closeclaw_common::MetricsEmitter), allowing call
//! sites to always invoke `emit_*` methods without null-checks when no
//! metrics backend is configured.

use closeclaw_common::{llm_stats::CacheBreakInfo, MetricsEmitter};

/// No-op metrics emitter — the default implementation.
///
/// All methods are empty; used when no metrics backend is configured.
pub struct NoopMetricsEmitter;

impl MetricsEmitter for NoopMetricsEmitter {
    fn emit_cache_break(&self, _info: &CacheBreakInfo) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that `NoopMetricsEmitter` does not panic and can be
    /// called with a real `CacheBreakInfo`.
    #[test]
    fn test_noop_metrics_emitter_cache_break_does_not_panic() {
        let emitter = NoopMetricsEmitter;
        let info = CacheBreakInfo {
            previous_cache_read: 10_000,
            current_cache_read: 5_000,
            drop_tokens: 5_000,
            drop_ratio: 0.5,
            previous_hit_rate: 0.5,
            current_hit_rate: 0.25,
        };
        emitter.emit_cache_break(&info);
    }
}
