use std::time::Instant;

use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::middleware::MiddlewareContext;

use super::rate_limit::{RateLimitMiddleware, SlidingWindow};
use closeclaw_common::middleware::OutboundMiddleware;

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
        closeclaw_common::middleware::MiddlewareError::Rejected { name, reason } => {
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
