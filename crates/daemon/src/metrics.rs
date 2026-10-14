//! Metrics emission defaults used by the daemon composition root.
//!
//! Provides [`NoopMetricsEmitter`] — a zero-cost no-op backend for
//! [`MetricsEmitter`](closeclaw_common::MetricsEmitter), allowing call
//! sites to always invoke `emit_*` methods without null-checks when no
//! metrics backend is configured.

use closeclaw_common::{llm_stats::CacheAnomalyInfo, MetricsEmitter};

/// No-op metrics emitter — the default implementation.
///
/// All methods are empty; used when no metrics backend is configured.
pub(crate) struct NoopMetricsEmitter;

impl MetricsEmitter for NoopMetricsEmitter {
    fn emit_cache_anomaly(&self, _info: &CacheAnomalyInfo) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that `NoopMetricsEmitter` does not panic and can be
    /// called with a real `CacheAnomalyInfo`.
    #[test]
    fn test_noop_metrics_emitter_cache_anomaly_does_not_panic() {
        let emitter = NoopMetricsEmitter;
        let info = CacheAnomalyInfo {
            previous_prompt_tokens: 10_000,
            current_cache_read_tokens: 5_000,
            deviation_tokens: 5_000,
            tolerance_tokens: 1024,
        };
        emitter.emit_cache_anomaly(&info);
    }
}
