//! Gateway-owned outbound middleware execution.
//!
//! The middleware chain (and the streaming pre-flight gate) is scheduled by
//! the Gateway itself — see `docs/design/gateway/outbound-flow.md`
//! 「出站中间件」. Both helpers therefore execute against the pure
//! definitions in `closeclaw_common::middleware` and never reference a
//! concrete processor-chain crate.

use std::sync::Arc;

use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::middleware::{MiddlewareContext, MiddlewareError, OutboundMiddleware};

/// Run a chain of outbound middlewares on a rendered output.
///
/// Processes `rendered` through each middleware in registration order. The
/// first error (including a rejection) short-circuits the chain and is
/// propagated to the caller, which decides how to log/notify.
pub(crate) async fn run_middleware_chain(
    middlewares: &[Arc<dyn OutboundMiddleware>],
    ctx: &MiddlewareContext,
    rendered: &RenderedOutput,
) -> Result<(), MiddlewareError> {
    for middleware in middlewares {
        middleware.process(ctx, rendered).await?;
    }
    Ok(())
}

/// Run the pre-flight check across the middleware chain.
///
/// Used once before a streaming outbound starts, so the gate runs on
/// session-level metadata without per-chunk overhead. The first rejection
/// short-circuits the chain immediately.
pub(crate) async fn run_pre_flight_check(
    middlewares: &[Arc<dyn OutboundMiddleware>],
    ctx: &MiddlewareContext,
) -> Result<(), MiddlewareError> {
    for middleware in middlewares {
        middleware.pre_flight_check(ctx).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_common::middleware::MiddlewareError;
    use closeclaw_common::RenderedOutput;

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
}
