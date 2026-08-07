//! Outbound middleware extension point.
//!
//! Provides the [`run_middleware_chain`] function that executes a chain
//! of [`OutboundMiddleware`]s on a rendered output.
//!
//! The [`OutboundMiddleware`] trait and [`MiddlewareError`] type are
//! defined in [`closeclaw_common::middleware`] (pure definitions).

pub use closeclaw_common::middleware::{MiddlewareError, OutboundMiddleware};

use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::MiddlewareContext;

/// Run a chain of outbound middlewares on a rendered output.
///
/// Processes `rendered` through each middleware in order. If any middleware
/// returns an error (including rejection), the chain short-circuits and
/// the error is propagated.
pub async fn run_middleware_chain(
    middlewares: &[std::sync::Arc<dyn OutboundMiddleware>],
    ctx: &MiddlewareContext,
    rendered: &RenderedOutput,
) -> Result<(), MiddlewareError> {
    for mw in middlewares {
        mw.process(ctx, rendered).await?;
    }
    Ok(())
}

/// Run pre-flight checks across the middleware chain.
///
/// Calls [`OutboundMiddleware::pre_flight_check`] on each middleware
/// using only session-level metadata. Used before streaming outbound
/// to gate the session without per-chunk overhead. If any middleware
/// rejects, the chain short-circuits immediately.
pub async fn run_pre_flight_check(
    middlewares: &[std::sync::Arc<dyn OutboundMiddleware>],
    ctx: &MiddlewareContext,
) -> Result<(), MiddlewareError> {
    for mw in middlewares {
        mw.pre_flight_check(ctx).await?;
    }
    Ok(())
}
