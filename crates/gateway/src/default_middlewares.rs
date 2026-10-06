//! Built-in outbound middlewares registered on every Gateway.

use std::sync::Arc;

use super::outbound_middleware;
use super::Gateway;
use super::GatewayConfig;

/// Register the built-in outbound middlewares on a [`Gateway`].
///
/// Every newly constructed Gateway receives:
/// - [`AuditMiddleware`] — logs every outbound message for audit.
/// - [`RateLimitMiddleware`] — session-level sliding-window throttling.
///
/// When `config.rate_limit_per_minute` is > 0 it is used as the per-session
/// message limit; otherwise the default (30) applies.
pub fn register_default_middlewares(gw: &Gateway, config: &GatewayConfig) {
    gw.add_outbound_middleware(Arc::new(outbound_middleware::audit::AuditMiddleware));
    let limit = if config.rate_limit_per_minute > 0 {
        config.rate_limit_per_minute as usize
    } else {
        outbound_middleware::rate_limit::DEFAULT_MAX_PER_MINUTE
    };
    gw.add_outbound_middleware(Arc::new(
        outbound_middleware::rate_limit::RateLimitMiddleware::with_limit(limit),
    ));
}
