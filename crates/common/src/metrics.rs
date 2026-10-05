//! Metrics emission abstraction.
//!
//! Defines the [`MetricsEmitter`] trait as a DI interface for reporting
//! operational metrics (e.g. cache breaks).
//!
//! Future backends (Prometheus, Grafana, etc.) only need to implement
//! this trait — no call-site changes required.

use crate::llm_stats::CacheBreakInfo;

/// Trait for emitting operational metrics.
///
/// This is a **DI trait** (归属 gateway 领域) placed in `closeclaw-common`
/// so that both the gateway crate and the daemon composition root can
/// depend on it without creating circular dependencies.
pub trait MetricsEmitter: Send + Sync {
    /// Record a KV cache break event.
    fn emit_cache_break(&self, info: &CacheBreakInfo);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that a mock `MetricsEmitter` receives the correct info.
    #[test]
    fn test_mock_metrics_emitter_receives_cache_break() {
        use std::sync::atomic::{AtomicBool, Ordering};

        struct MockEmitter {
            called: AtomicBool,
        }

        impl MetricsEmitter for MockEmitter {
            fn emit_cache_break(&self, _info: &CacheBreakInfo) {
                self.called.store(true, Ordering::Relaxed);
            }
        }

        let emitter = MockEmitter {
            called: AtomicBool::new(false),
        };
        assert!(!emitter.called.load(Ordering::Relaxed));

        let info = CacheBreakInfo {
            previous_cache_read: 20_000,
            current_cache_read: 1_000,
            drop_tokens: 19_000,
            drop_ratio: 0.95,
            previous_hit_rate: 0.8,
            current_hit_rate: 0.05,
        };
        emitter.emit_cache_break(&info);
        assert!(emitter.called.load(Ordering::Relaxed));
    }
}
