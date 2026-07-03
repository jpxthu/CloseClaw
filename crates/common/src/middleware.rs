//! Outbound middleware extension point.
//!
//! Defines the [`OutboundMiddleware`] trait that allows intercepting
//! rendered outbound messages between IM Adapter rendering and sending.
//!
//! The middleware chain runs after [`IMPlugin::render`] produces a
//! [`RenderedOutput`] and before [`IMPlugin::send`] delivers it to
//! the target platform.

use async_trait::async_trait;
use thiserror::Error;

use crate::im_plugin::RenderedOutput;

// ---------------------------------------------------------------------------
// MiddlewareError
// ---------------------------------------------------------------------------

/// Errors raised during middleware processing.
#[derive(Debug, Error)]
pub enum MiddlewareError {
    /// A middleware in the chain failed.
    #[error("middleware `{name}` failed: {source}")]
    MiddlewareFailed {
        name: String,
        #[source]
        source: anyhow::Error,
    },
}

impl MiddlewareError {
    /// Constructs a `MiddlewareFailed` error.
    #[inline]
    pub fn middleware_failed(name: impl Into<String>, source: impl std::fmt::Display) -> Self {
        Self::MiddlewareFailed {
            name: name.into(),
            source: anyhow::Error::msg(source.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// OutboundMiddleware trait
// ---------------------------------------------------------------------------

/// Middleware that intercepts rendered outbound messages.
///
/// Implementations can inspect, modify, or reject outbound messages
/// before they are sent to the target platform. The middleware chain
/// runs between [`IMPlugin::render`] and [`IMPlugin::send`].
///
/// # Examples
///
/// ```ignore
/// use closeclaw_common::middleware::OutboundMiddleware;
/// use closeclaw_common::im_plugin::RenderedOutput;
///
/// struct LoggingMiddleware;
///
/// #[async_trait]
/// impl OutboundMiddleware for LoggingMiddleware {
///     fn name(&self) -> &str {
///         "logging"
///     }
///
///     async fn process(
///         &self,
///         rendered: &RenderedOutput,
///     ) -> Result<RenderedOutput, MiddlewareError> {
///         tracing::info!("outbound message type={}", rendered.msg_type);
///         Ok(rendered.clone())
///     }
/// }
/// ```
#[async_trait]
pub trait OutboundMiddleware: Send + Sync {
    /// Return the middleware's name for error reporting and logging.
    fn name(&self) -> &str;

    /// Process the rendered output before it is sent.
    ///
    /// Returning `Ok(rendered)` passes the (possibly modified) output
    /// to the next middleware in the chain. Returning `Err` short-circuits
    /// the chain and aborts the send.
    async fn process(&self, rendered: &RenderedOutput) -> Result<RenderedOutput, MiddlewareError>;
}

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
