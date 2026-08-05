//! Unit tests for outbound middleware extension point.
//!
//! Verifies:
//! - `OutboundMiddleware` trait contract via mock implementations
//! - `run_middleware_chain` execution order and passthrough
//! - Middleware called after render (i.e., receives RenderedOutput)

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use crate::middleware::{run_middleware_chain, MiddlewareError, OutboundMiddleware};
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::MiddlewareContext;

// ---------------------------------------------------------------------------
// Mock middlewares
// ---------------------------------------------------------------------------

/// Mock middleware that records how many times `process` is called
/// and always allows the message.
struct PassthroughMiddleware {
    name: String,
    call_count: Arc<AtomicUsize>,
}

impl PassthroughMiddleware {
    fn new(name: &str) -> (Self, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        (
            Self {
                name: name.to_string(),
                call_count: counter.clone(),
            },
            counter,
        )
    }
}

#[async_trait]
impl OutboundMiddleware for PassthroughMiddleware {
    fn name(&self) -> &str {
        &self.name
    }

    async fn process(
        &self,
        _ctx: &MiddlewareContext,
        _rendered: &RenderedOutput,
    ) -> Result<(), MiddlewareError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Mock middleware that always rejects, short-circuiting the chain.
struct RejectingMiddleware;

#[async_trait]
impl OutboundMiddleware for RejectingMiddleware {
    fn name(&self) -> &str {
        "rejecting"
    }

    async fn process(
        &self,
        _ctx: &MiddlewareContext,
        _rendered: &RenderedOutput,
    ) -> Result<(), MiddlewareError> {
        Err(MiddlewareError::rejected("rejecting", "intentional reject"))
    }
}

fn sample_rendered() -> RenderedOutput {
    RenderedOutput {
        msg_type: "text".to_string(),
        payload: serde_json::json!({"content": {"text": "hello"}}),
    }
}

fn sample_ctx() -> MiddlewareContext {
    MiddlewareContext {
        session_id: "sess-1".to_string(),
        channel: "feishu".to_string(),
        chat_id: "chat-1".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_single_middleware_called() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mw, counter) = PassthroughMiddleware::new("mw1");
    let middlewares: Vec<Arc<dyn OutboundMiddleware>> = vec![Arc::new(mw)];
    let ctx = sample_ctx();
    let rendered = sample_rendered();

    rt.block_on(run_middleware_chain(&middlewares, &ctx, &rendered))
        .unwrap();

    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[test]
fn test_empty_chain_passthrough() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let middlewares: Vec<Arc<dyn OutboundMiddleware>> = vec![];
    let ctx = sample_ctx();
    let rendered = sample_rendered();

    rt.block_on(run_middleware_chain(&middlewares, &ctx, &rendered))
        .unwrap();
}

#[test]
fn test_multiple_middlewares_called_in_order() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mw1, c1) = PassthroughMiddleware::new("first");
    let (mw2, c2) = PassthroughMiddleware::new("second");
    let (mw3, c3) = PassthroughMiddleware::new("third");

    let middlewares: Vec<Arc<dyn OutboundMiddleware>> =
        vec![Arc::new(mw1), Arc::new(mw2), Arc::new(mw3)];
    let ctx = sample_ctx();
    let rendered = sample_rendered();

    rt.block_on(run_middleware_chain(&middlewares, &ctx, &rendered))
        .unwrap();

    assert_eq!(c1.load(Ordering::SeqCst), 1);
    assert_eq!(c2.load(Ordering::SeqCst), 1);
    assert_eq!(c3.load(Ordering::SeqCst), 1);
}

#[test]
fn test_rejecting_middleware_short_circuits() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mw_ok, c_ok) = PassthroughMiddleware::new("ok");
    let middlewares: Vec<Arc<dyn OutboundMiddleware>> =
        vec![Arc::new(mw_ok), Arc::new(RejectingMiddleware)];
    let ctx = sample_ctx();
    let rendered = sample_rendered();

    let err = rt
        .block_on(run_middleware_chain(&middlewares, &ctx, &rendered))
        .unwrap_err();

    assert_eq!(c_ok.load(Ordering::SeqCst), 1);
    match err {
        MiddlewareError::Rejected { name, reason } => {
            assert_eq!(name, "rejecting");
            assert_eq!(reason, "intentional reject");
        }
        _ => panic!("expected Rejected variant"),
    }
}

#[test]
fn test_rejecting_middleware_prevents_subsequent_middlewares() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mw_late, c_late) = PassthroughMiddleware::new("late");
    let middlewares: Vec<Arc<dyn OutboundMiddleware>> =
        vec![Arc::new(RejectingMiddleware), Arc::new(mw_late)];
    let ctx = sample_ctx();
    let rendered = sample_rendered();

    let _ = rt.block_on(run_middleware_chain(&middlewares, &ctx, &rendered));

    assert_eq!(c_late.load(Ordering::SeqCst), 0);
}

#[test]
fn test_middleware_receives_rendered_output() {
    let rt = tokio::runtime::Runtime::new().unwrap();

    struct VerifyRenderedMiddleware;

    #[async_trait]
    impl OutboundMiddleware for VerifyRenderedMiddleware {
        fn name(&self) -> &str {
            "verify_rendered"
        }

        async fn process(
            &self,
            _ctx: &MiddlewareContext,
            rendered: &RenderedOutput,
        ) -> Result<(), MiddlewareError> {
            assert!(!rendered.msg_type.is_empty());
            Ok(())
        }
    }

    let middlewares: Vec<Arc<dyn OutboundMiddleware>> = vec![Arc::new(VerifyRenderedMiddleware)];
    let ctx = sample_ctx();
    let rendered = RenderedOutput {
        msg_type: "text".to_string(),
        payload: serde_json::json!({"content": {"text": "hello"}}),
    };

    rt.block_on(run_middleware_chain(&middlewares, &ctx, &rendered))
        .unwrap();
}

#[test]
fn test_middleware_receives_context() {
    let rt = tokio::runtime::Runtime::new().unwrap();

    struct CtxCheckMiddleware;

    #[async_trait]
    impl OutboundMiddleware for CtxCheckMiddleware {
        fn name(&self) -> &str {
            "ctx_check"
        }

        async fn process(
            &self,
            ctx: &MiddlewareContext,
            _rendered: &RenderedOutput,
        ) -> Result<(), MiddlewareError> {
            assert_eq!(ctx.session_id, "sess-1");
            assert_eq!(ctx.channel, "feishu");
            assert_eq!(ctx.chat_id, "chat-1");
            Ok(())
        }
    }

    let middlewares: Vec<Arc<dyn OutboundMiddleware>> = vec![Arc::new(CtxCheckMiddleware)];
    let ctx = sample_ctx();
    let rendered = sample_rendered();

    rt.block_on(run_middleware_chain(&middlewares, &ctx, &rendered))
        .unwrap();
}
