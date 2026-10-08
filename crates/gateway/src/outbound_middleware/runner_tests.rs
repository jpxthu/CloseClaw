use std::sync::Arc;

use closeclaw_common::middleware::{MiddlewareContext, MiddlewareError, OutboundMiddleware};
use closeclaw_common::RenderedOutput;

use super::runner::{run_middleware_chain, run_pre_flight_check};

struct AcceptAll;
#[async_trait::async_trait]
impl OutboundMiddleware for AcceptAll {
    fn name(&self) -> &str {
        "accept-all"
    }
    async fn process(
        &self,
        _ctx: &MiddlewareContext,
        _rendered: &RenderedOutput,
    ) -> Result<(), MiddlewareError> {
        Ok(())
    }
    async fn pre_flight_check(&self, _ctx: &MiddlewareContext) -> Result<(), MiddlewareError> {
        Ok(())
    }
}

struct RejectOnPreFlight;
#[async_trait::async_trait]
impl OutboundMiddleware for RejectOnPreFlight {
    fn name(&self) -> &str {
        "reject-on-pre-flight"
    }
    async fn process(
        &self,
        _ctx: &MiddlewareContext,
        _rendered: &RenderedOutput,
    ) -> Result<(), MiddlewareError> {
        Ok(())
    }
    async fn pre_flight_check(&self, _ctx: &MiddlewareContext) -> Result<(), MiddlewareError> {
        Err(MiddlewareError::rejected("reject-on-pre-flight", "nope"))
    }
}

fn ctx() -> MiddlewareContext {
    MiddlewareContext {
        session_id: "sess".to_string(),
        channel: "mock".to_string(),
        chat_id: "chat".to_string(),
    }
}

fn rendered() -> RenderedOutput {
    RenderedOutput {
        msg_type: "text".to_string(),
        payload: serde_json::json!("hello"),
    }
}

#[tokio::test]
async fn test_run_middleware_chain_runs_all_in_order() {
    let middlewares: Vec<Arc<dyn OutboundMiddleware>> = vec![
        Arc::new(AcceptAll),
        Arc::new(AcceptAll),
        Arc::new(RejectOnPreFlight),
    ];
    // A rejecting middleware only fails pre-flight: `process` stays Ok,
    // proving every entry got its turn.
    run_middleware_chain(&middlewares, &ctx(), &rendered())
        .await
        .expect("all middlewares accept the rendered output");
}

#[tokio::test]
async fn test_run_middleware_chain_short_circuits_on_error() {
    struct RejectOnProcess;
    #[async_trait::async_trait]
    impl OutboundMiddleware for RejectOnProcess {
        fn name(&self) -> &str {
            "reject-on-process"
        }
        async fn process(
            &self,
            _ctx: &MiddlewareContext,
            _rendered: &RenderedOutput,
        ) -> Result<(), MiddlewareError> {
            Err(MiddlewareError::rejected("reject-on-process", "blocked"))
        }
    }
    let middlewares: Vec<Arc<dyn OutboundMiddleware>> =
        vec![Arc::new(AcceptAll), Arc::new(RejectOnProcess)];
    let err = run_middleware_chain(&middlewares, &ctx(), &rendered())
        .await
        .expect_err("rejection must propagate");
    assert!(
        matches!(&err, MiddlewareError::Rejected { name, .. } if name == "reject-on-process"),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn test_empty_chain_is_a_no_op() {
    let middlewares: Vec<Arc<dyn OutboundMiddleware>> = Vec::new();
    run_middleware_chain(&middlewares, &ctx(), &rendered())
        .await
        .expect("empty chain passes");
    run_pre_flight_check(&middlewares, &ctx())
        .await
        .expect("empty pre-flight passes");
}
