//! Rate-limit middleware — session-level sliding-window throttling.
//!
//! Each session is limited to a configurable number of outbound messages
//! per 60-second sliding window (default 30). When the limit is
//! exceeded the message is rejected with [`MiddlewareError::Rejected`].

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::middleware::{MiddlewareContext, MiddlewareError, OutboundMiddleware};
use tokio::sync::RwLock;

/// Default maximum messages per session per 60-second window.
const DEFAULT_MAX_PER_MINUTE: usize = 30;

/// Sliding window that tracks message send timestamps for one session.
pub(crate) struct SlidingWindow {
    pub(crate) timestamps: Vec<Instant>,
}

impl SlidingWindow {
    pub(crate) fn new() -> Self {
        Self {
            timestamps: Vec::new(),
        }
    }

    /// Remove entries older than 60 seconds, then return remaining count.
    pub(crate) fn prune_and_count(&mut self) -> usize {
        let cutoff = Instant::now() - std::time::Duration::from_secs(60);
        self.timestamps.retain(|&t| t > cutoff);
        self.timestamps.len()
    }

    fn record(&mut self) {
        self.timestamps.push(Instant::now());
    }
}

/// Session-level sliding-window rate limiter for outbound messages.
///
/// Tracks per-session message counts using a 60-second sliding window.
/// When a session exceeds `max_per_minute` (default 30) messages
/// within the window the message is rejected.
pub struct RateLimitMiddleware {
    pub(crate) max_per_minute: usize,
    pub(crate) windows: Arc<RwLock<HashMap<String, SlidingWindow>>>,
}

impl Default for RateLimitMiddleware {
    fn default() -> Self {
        Self::with_limit(DEFAULT_MAX_PER_MINUTE)
    }
}

impl RateLimitMiddleware {
    /// Create a new rate limiter with the default limit (30 msg/min).
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new rate limiter with a custom per-session limit.
    pub fn with_limit(max_per_minute: usize) -> Self {
        Self {
            max_per_minute,
            windows: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl OutboundMiddleware for RateLimitMiddleware {
    fn name(&self) -> &str {
        "rate_limit"
    }

    async fn process(
        &self,
        ctx: &MiddlewareContext,
        _rendered: &RenderedOutput,
    ) -> Result<(), MiddlewareError> {
        self.check_rate_limit(ctx).await
    }

    async fn pre_flight_check(&self, ctx: &MiddlewareContext) -> Result<(), MiddlewareError> {
        self.check_rate_limit(ctx).await
    }
}

impl RateLimitMiddleware {
    /// Core rate-limit check shared by both `process` and `pre_flight_check`.
    async fn check_rate_limit(&self, ctx: &MiddlewareContext) -> Result<(), MiddlewareError> {
        let mut windows = self.windows.write().await;
        let window = windows
            .entry(ctx.session_id.clone())
            .or_insert_with(SlidingWindow::new);

        let current = window.prune_and_count();

        if current >= self.max_per_minute {
            return Err(MiddlewareError::rejected(
                "rate_limit",
                format!(
                    "session {} exceeded rate limit: {}/{} messages in 60s",
                    ctx.session_id, current, self.max_per_minute,
                ),
            ));
        }

        window.record();
        Ok(())
    }
}
