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
use closeclaw_debug_log::DebugLog;

use crate::debug_log::{
    emit_processor_chain_event, ProcessorChainDebugLogContext, ProcessorChainEmitEventParams,
};

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
    run_middleware_chain_with_debug(None, middlewares, ctx, rendered).await
}

/// Run a chain of outbound middlewares with optional debug-log emission.
///
/// When `debug_log` is `Some`, emits `chain.middleware` events (中间状态)
/// for each middleware execution.
pub async fn run_middleware_chain_with_debug(
    debug_log: Option<&DebugLog>,
    middlewares: &[std::sync::Arc<dyn OutboundMiddleware>],
    ctx: &MiddlewareContext,
    rendered: &RenderedOutput,
) -> Result<(), MiddlewareError> {
    for mw in middlewares {
        if let Some(dl) = debug_log {
            let log_ctx = ProcessorChainDebugLogContext::new(Some(dl), &ctx.session_id, None);
            emit_processor_chain_event(ProcessorChainEmitEventParams {
                ctx: log_ctx,
                level: closeclaw_debug_log::LogLevel::Debug,
                source_module: "processor_chain",
                event_type: "chain.middleware",
                payload: serde_json::json!({
                    "middleware": mw.name(),
                    "phase": "process",
                }),
                parent: None,
            });
        }
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
    run_pre_flight_check_with_debug(None, middlewares, ctx).await
}

/// Run pre-flight checks with optional debug-log emission.
///
/// When `debug_log` is `Some`, emits `chain.middleware` events (中间状态)
/// for each middleware pre-flight check.
pub async fn run_pre_flight_check_with_debug(
    debug_log: Option<&DebugLog>,
    middlewares: &[std::sync::Arc<dyn OutboundMiddleware>],
    ctx: &MiddlewareContext,
) -> Result<(), MiddlewareError> {
    for mw in middlewares {
        if let Some(dl) = debug_log {
            let log_ctx = ProcessorChainDebugLogContext::new(Some(dl), &ctx.session_id, None);
            emit_processor_chain_event(ProcessorChainEmitEventParams {
                ctx: log_ctx,
                level: closeclaw_debug_log::LogLevel::Debug,
                source_module: "processor_chain",
                event_type: "chain.middleware",
                payload: serde_json::json!({
                    "middleware": mw.name(),
                    "phase": "pre_flight_check",
                }),
                parent: None,
            });
        }
        mw.pre_flight_check(ctx).await?;
    }
    Ok(())
}
