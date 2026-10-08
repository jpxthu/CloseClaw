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
