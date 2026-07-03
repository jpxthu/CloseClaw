//! Outbound middleware extension point.
//!
//! Provides the [`run_middleware_chain`] function that executes a chain
//! of [`OutboundMiddleware`]s on a rendered output.
//!
//! The [`OutboundMiddleware`] trait and [`MiddlewareError`] type are
//! defined in [`closeclaw_common::middleware`] (pure definitions).

pub use closeclaw_common::middleware::{MiddlewareError, OutboundMiddleware};

use closeclaw_common::im_plugin::RenderedOutput;

/// Run a chain of outbound middlewares on a rendered output.
///
/// Processes `rendered` through each middleware in order. If any middleware
/// returns an error, the chain short-circuits and the error is propagated.
pub async fn run_middleware_chain(
    middlewares: &[std::sync::Arc<dyn OutboundMiddleware>],
    rendered: RenderedOutput,
) -> Result<RenderedOutput, MiddlewareError> {
    let mut current = rendered;
    for mw in middlewares {
        current = mw.process(&current).await?;
    }
    Ok(current)
}
