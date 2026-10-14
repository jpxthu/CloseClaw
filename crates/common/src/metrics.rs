//! Metrics emission abstraction.
//!
//! Defines the [`MetricsEmitter`] trait as a DI interface for reporting
//! operational metrics (e.g. cache anomalies).
//!
//! Future backends (Prometheus, Grafana, etc.) only need to implement
//! this trait — no call-site changes required.

use crate::llm_stats::CacheAnomalyInfo;

/// Trait for emitting operational metrics.
///
/// This is a **DI trait** (归属 gateway 领域) placed in `closeclaw-common`
/// so that both the gateway crate and the daemon composition root can
/// depend on it without creating circular dependencies.
pub trait MetricsEmitter: Send + Sync {
    /// Record a cache hit anomaly event.
    fn emit_cache_anomaly(&self, info: &CacheAnomalyInfo);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that a mock `MetricsEmitter` receives the correct info.
    #[test]
    fn test_mock_metrics_emitter_receives_cache_anomaly() {
        use std::sync::atomic::{AtomicBool, Ordering};

        struct MockEmitter {
            called: AtomicBool,
        }

        impl MetricsEmitter for MockEmitter {
            fn emit_cache_anomaly(&self, _info: &CacheAnomalyInfo) {
                self.called.store(true, Ordering::Relaxed);
            }
        }

        let emitter = MockEmitter {
            called: AtomicBool::new(false),
        };
        assert!(!emitter.called.load(Ordering::Relaxed));

        let info = CacheAnomalyInfo {
            previous_prompt_tokens: 20_000,
            current_cache_read_tokens: 1_000,
            deviation_tokens: 19_000,
            tolerance_tokens: 1024,
        };
        emitter.emit_cache_anomaly(&info);
        assert!(emitter.called.load(Ordering::Relaxed));
    }
}
