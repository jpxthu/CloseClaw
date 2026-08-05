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
#[derive(Debug)]
struct SlidingWindow {
    timestamps: Vec<Instant>,
}

impl SlidingWindow {
    fn new() -> Self {
        Self {
            timestamps: Vec::new(),
        }
    }

    /// Remove entries older than 60 seconds, then return remaining count.
    fn prune_and_count(&mut self) -> usize {
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
    max_per_minute: usize,
    windows: Arc<RwLock<HashMap<String, SlidingWindow>>>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx(session_id: &str) -> MiddlewareContext {
        MiddlewareContext {
            session_id: session_id.into(),
            channel: "feishu".into(),
            chat_id: "c1".into(),
        }
    }

    fn make_rendered() -> RenderedOutput {
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": "hi"}}),
        }
    }

    #[tokio::test]
    async fn test_within_limit_allows() {
        let mw = RateLimitMiddleware::with_limit(5);
        let ctx = make_ctx("s1");
        let rendered = make_rendered();

        // Send 5 messages — all should succeed.
        for _ in 0..5 {
            assert!(mw.process(&ctx, &rendered).await.is_ok());
        }
    }

    #[tokio::test]
    async fn test_exceeds_limit_rejects() {
        let mw = RateLimitMiddleware::with_limit(3);
        let ctx = make_ctx("s1");
        let rendered = make_rendered();

        for _ in 0..3 {
            assert!(mw.process(&ctx, &rendered).await.is_ok());
        }
        let result = mw.process(&ctx, &rendered).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            MiddlewareError::Rejected { name, reason } => {
                assert_eq!(name, "rate_limit");
                assert!(reason.contains("exceeded rate limit"));
            }
            other => panic!("expected Rejected, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_different_sessions_independent() {
        let mw = RateLimitMiddleware::with_limit(2);
        let rendered = make_rendered();
        let ctx1 = make_ctx("s1");
        let ctx2 = make_ctx("s2");

        // Fill s1 to limit.
        assert!(mw.process(&ctx1, &rendered).await.is_ok());
        assert!(mw.process(&ctx1, &rendered).await.is_ok());
        assert!(mw.process(&ctx1, &rendered).await.is_err());

        // s2 should still be fine.
        assert!(mw.process(&ctx2, &rendered).await.is_ok());
        assert!(mw.process(&ctx2, &rendered).await.is_ok());
        assert!(mw.process(&ctx2, &rendered).await.is_err());
    }

    #[tokio::test]
    async fn test_window_expires() {
        let mw = RateLimitMiddleware::with_limit(2);
        let ctx = make_ctx("s1");
        let rendered = make_rendered();

        // Fill to limit.
        assert!(mw.process(&ctx, &rendered).await.is_ok());
        assert!(mw.process(&ctx, &rendered).await.is_ok());
        assert!(mw.process(&ctx, &rendered).await.is_err());

        // Manually expire the window by overwriting with old timestamps.
        {
            let mut windows = mw.windows.write().await;
            if let Some(w) = windows.get_mut("s1") {
                let old = Instant::now() - std::time::Duration::from_secs(61);
                w.timestamps = vec![old; 2];
            }
        }

        // After expiry, new message should be allowed.
        assert!(mw.process(&ctx, &rendered).await.is_ok());
    }

    #[test]
    fn test_sliding_window_prune() {
        let mut w = SlidingWindow::new();
        let now = Instant::now();
        let old = now - std::time::Duration::from_secs(65);
        let recent = now - std::time::Duration::from_secs(10);

        w.timestamps = vec![old, old, recent];
        let count = w.prune_and_count();
        assert_eq!(count, 1, "only recent timestamp should remain");
    }

    #[test]
    fn test_default_limit() {
        let mw = RateLimitMiddleware::new();
        assert_eq!(mw.max_per_minute, 30);
    }
}
