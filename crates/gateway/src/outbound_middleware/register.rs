//! Registration of the built-in outbound middlewares.

use std::sync::Arc;

use super::{audit, rate_limit};
use crate::{Gateway, GatewayConfig};

/// Register the built-in outbound middlewares on a [`Gateway`].
///
/// Every newly constructed Gateway receives:
/// - [`audit::AuditMiddleware`] — logs every outbound message for audit.
/// - [`rate_limit::RateLimitMiddleware`] — session-level sliding-window
///   throttling.
///
/// When `config.rate_limit_per_minute` is > 0 it is used as the per-session
/// message limit; otherwise the default (30) applies.
pub(crate) fn register_default(gw: &Gateway, config: &GatewayConfig) {
    gw.add_outbound_middleware(Arc::new(audit::AuditMiddleware));
    let limit = if config.rate_limit_per_minute > 0 {
        config.rate_limit_per_minute as usize
    } else {
        rate_limit::DEFAULT_MAX_PER_MINUTE
    };
    gw.add_outbound_middleware(Arc::new(rate_limit::RateLimitMiddleware::with_limit(limit)));
}
