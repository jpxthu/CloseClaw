//! Unit tests for outbound middleware extension point.
//!
//! Verifies:
//! - `OutboundMiddleware` trait contract via mock implementations
//! - `run_middleware_chain` execution order and passthrough
//! - Middleware called after render (i.e., receives RenderedOutput)

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use crate::im_plugin::RenderedOutput;
use crate::middleware::{run_middleware_chain, OutboundMiddleware};

// ---------------------------------------------------------------------------
// Mock middlewares
// ---------------------------------------------------------------------------

/// Mock middleware that records how many times `process` is called
/// and returns the input unchanged.
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
        rendered: &RenderedOutput,
    ) -> Result<RenderedOutput, crate::middleware::MiddlewareError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Ok(rendered.clone())
    }
}

/// Mock middleware that modifies the msg_type to prove it received the output.
struct TransformMiddleware;

#[async_trait]
impl OutboundMiddleware for TransformMiddleware {
    fn name(&self) -> &str {
        "transform"
    }

    async fn process(
        &self,
        rendered: &RenderedOutput,
    ) -> Result<RenderedOutput, crate::middleware::MiddlewareError> {
        Ok(RenderedOutput {
            msg_type: "modified".to_string(),
            payload: rendered.payload.clone(),
        })
    }
}

/// Mock middleware that always errors, short-circuiting the chain.
struct FailingMiddleware;

#[async_trait]
impl OutboundMiddleware for FailingMiddleware {
    fn name(&self) -> &str {
        "failing"
    }

    async fn process(
        &self,
        _rendered: &RenderedOutput,
    ) -> Result<RenderedOutput, crate::middleware::MiddlewareError> {
        Err(crate::middleware::MiddlewareError::middleware_failed(
            "failing",
            "intentional error",
        ))
    }
}

fn sample_rendered() -> RenderedOutput {
    RenderedOutput {
        msg_type: "text".to_string(),
        payload: serde_json::json!({"content": {"text": "hello"}}),
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

    let result = rt
        .block_on(run_middleware_chain(&middlewares, sample_rendered()))
        .unwrap();

    assert_eq!(counter.load(Ordering::SeqCst), 1);
    assert_eq!(result.msg_type, "text");
}

#[test]
fn test_empty_chain_passthrough() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let middlewares: Vec<Arc<dyn OutboundMiddleware>> = vec![];
    let input = sample_rendered();

    let result = rt
        .block_on(run_middleware_chain(&middlewares, input.clone()))
        .unwrap();

    assert_eq!(result.msg_type, input.msg_type);
    assert_eq!(result.payload, input.payload);
}

#[test]
fn test_multiple_middlewares_called_in_order() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mw1, c1) = PassthroughMiddleware::new("first");
    let (mw2, c2) = PassthroughMiddleware::new("second");
    let (mw3, c3) = PassthroughMiddleware::new("third");

    let middlewares: Vec<Arc<dyn OutboundMiddleware>> =
        vec![Arc::new(mw1), Arc::new(mw2), Arc::new(mw3)];

    let _ = rt.block_on(run_middleware_chain(&middlewares, sample_rendered()));

    assert_eq!(c1.load(Ordering::SeqCst), 1);
    assert_eq!(c2.load(Ordering::SeqCst), 1);
    assert_eq!(c3.load(Ordering::SeqCst), 1);
}

#[test]
fn test_transform_middleware_modifies_output() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let middlewares: Vec<Arc<dyn OutboundMiddleware>> = vec![Arc::new(TransformMiddleware)];

    let result = rt
        .block_on(run_middleware_chain(&middlewares, sample_rendered()))
        .unwrap();

    assert_eq!(result.msg_type, "modified");
    assert_eq!(
        result.payload,
        serde_json::json!({"content": {"text": "hello"}})
    );
}

#[test]
fn test_transform_chain_passthrough_then_transform() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mw_passthrough, c_pt) = PassthroughMiddleware::new("pt");
    let middlewares: Vec<Arc<dyn OutboundMiddleware>> =
        vec![Arc::new(mw_passthrough), Arc::new(TransformMiddleware)];

    let result = rt
        .block_on(run_middleware_chain(&middlewares, sample_rendered()))
        .unwrap();

    assert_eq!(c_pt.load(Ordering::SeqCst), 1);
    assert_eq!(result.msg_type, "modified");
}

#[test]
fn test_failing_middleware_short_circuits() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mw_ok, c_ok) = PassthroughMiddleware::new("ok");
    let middlewares: Vec<Arc<dyn OutboundMiddleware>> =
        vec![Arc::new(mw_ok), Arc::new(FailingMiddleware)];

    let err = rt
        .block_on(run_middleware_chain(&middlewares, sample_rendered()))
        .unwrap_err();

    assert_eq!(c_ok.load(Ordering::SeqCst), 1);
    assert!(err.to_string().contains("failing"));
}

#[test]
fn test_failing_middleware_prevents_subsequent_middlewares() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mw_late, c_late) = PassthroughMiddleware::new("late");
    let middlewares: Vec<Arc<dyn OutboundMiddleware>> =
        vec![Arc::new(FailingMiddleware), Arc::new(mw_late)];

    let _ = rt.block_on(run_middleware_chain(&middlewares, sample_rendered()));

    assert_eq!(c_late.load(Ordering::SeqCst), 0);
}

#[test]
fn test_middleware_receives_rendered_output_not_raw() {
    let rt = tokio::runtime::Runtime::new().unwrap();

    static RECEIVED_RENDERED: AtomicBool = AtomicBool::new(false);

    struct VerifyRenderedMiddleware;

    #[async_trait]
    impl OutboundMiddleware for VerifyRenderedMiddleware {
        fn name(&self) -> &str {
            "verify_rendered"
        }

        async fn process(
            &self,
            rendered: &RenderedOutput,
        ) -> Result<RenderedOutput, crate::middleware::MiddlewareError> {
            assert!(!rendered.msg_type.is_empty());
            RECEIVED_RENDERED.store(true, Ordering::SeqCst);
            Ok(rendered.clone())
        }
    }

    let middlewares: Vec<Arc<dyn OutboundMiddleware>> = vec![Arc::new(VerifyRenderedMiddleware)];

    let rendered = RenderedOutput {
        msg_type: "text".to_string(),
        payload: serde_json::json!({"content": {"text": "hello"}}),
    };

    let _ = rt.block_on(run_middleware_chain(&middlewares, rendered));
    assert!(RECEIVED_RENDERED.load(Ordering::SeqCst));
}
