//! Outbound middleware extension point.
//!
//! Defines the [`OutboundMiddleware`] trait that allows inspecting
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

    /// A middleware explicitly rejected the outbound message.
    #[error("middleware `{name}` rejected message: {reason}")]
    Rejected { name: String, reason: String },
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

    /// Constructs a `Rejected` error.
    #[inline]
    pub fn rejected(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Rejected {
            name: name.into(),
            reason: reason.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// MiddlewareContext
// ---------------------------------------------------------------------------

/// Context passed to outbound middlewares alongside the rendered output.
///
/// Provides session-level metadata that middlewares may need for
/// inspection, logging, or rate-limiting decisions. The context is
/// constructed by the Gateway before the middleware chain runs and is
/// passed as an immutable reference — middlewares must not attempt to
/// modify it.
#[derive(Debug, Clone)]
pub struct MiddlewareContext {
    /// The session identifier this message belongs to.
    pub session_id: String,
    /// The target IM channel (e.g. "feishu", "telegram").
    pub channel: String,
    /// The chat / group identifier on the target platform.
    pub chat_id: String,
}

// ---------------------------------------------------------------------------
// OutboundMiddleware trait
// ---------------------------------------------------------------------------

/// Middleware that intercepts rendered outbound messages.
///
/// Implementations inspect outbound messages and decide whether to
/// allow or reject them. The middleware chain runs between
/// [`IMPlugin::render`] and [`IMPlugin::send`]. Middlewares must not
/// modify the message content — returning `Ok(())` signals "allow"
/// and returning `Err(MiddlewareError::Rejected { .. })` signals
/// "reject".
///
/// The [`MiddlewareContext`] provides session-level metadata (session ID,
/// channel, chat ID) that middlewares can use for decisions such as
/// rate limiting or audit logging.
///
/// # Examples
///
/// ```ignore
/// use closeclaw_common::middleware::{MiddlewareContext, OutboundMiddleware};
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
///         _ctx: &MiddlewareContext,
///         rendered: &RenderedOutput,
///     ) -> Result<(), MiddlewareError> {
///         tracing::info!("outbound message type={}", rendered.msg_type);
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait OutboundMiddleware: Send + Sync {
    /// Return the middleware's name for error reporting and logging.
    fn name(&self) -> &str;

    /// Inspect the rendered output and decide whether to allow or reject.
    ///
    /// Returning `Ok(())` allows the message to proceed to the next
    /// middleware or to the send phase. Returning
    /// `Err(MiddlewareError::Rejected { .. })` short-circuits the chain
    /// and the message is not sent.
    async fn process(
        &self,
        ctx: &MiddlewareContext,
        rendered: &RenderedOutput,
    ) -> Result<(), MiddlewareError>;
}
