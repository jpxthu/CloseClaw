//! Processor registry construction for the Gateway.
//!
//! Builds the standard inbound/outbound processor chains and registers
//! default outbound middlewares.

use std::sync::Arc;

use closeclaw_processor_chain::content_normalizer::ContentNormalizer;
use closeclaw_processor_chain::outbound_raw_log::OutboundRawLogProcessor;
use closeclaw_processor_chain::raw_log_processor::{RawLogConfig, RawLogProcessor};
use closeclaw_processor_chain::registry::ProcessorRegistry;
use closeclaw_processor_chain::session_router::SessionRouter;
use closeclaw_processor_chain::verbosity_filter::VerbosityFilter;
use closeclaw_processor_chain::DslParser;

use super::outbound_middleware;
use super::Gateway;
use super::GatewayConfig;

/// Build a [`ProcessorRegistry`] with the standard inbound/outbound chains.
///
/// Inbound (by priority): [`RawLogProcessor`] (10) → [`SessionRouter`] (20) →
/// [`ContentNormalizer`] (30).
///
/// Outbound (by priority): [`VerbosityFilter`] (5) → [`DslParser`] (10) →
/// [`OutboundRawLogProcessor`] (20, only when `raw_log_dir` is configured).
///
/// [`RawLogProcessor`] and [`OutboundRawLogProcessor`] are registered only
/// when `config.raw_log_dir` is `Some`.
pub fn build_processor_registry(config: &GatewayConfig) -> ProcessorRegistry {
    let mut registry = ProcessorRegistry::default();

    // Inbound: RawLogProcessor (priority 10 — if raw_log_dir is configured)
    if let Some(ref dir) = config.raw_log_dir {
        let raw_log_config = RawLogConfig {
            enabled: true,
            dir: dir.clone(),
            retention_days: 7,
        };
        match RawLogProcessor::new(raw_log_config) {
            Ok(processor) => {
                registry.register(Arc::new(processor));
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "RawLogProcessor initialization failed — skipping inbound raw log processor"
                );
            }
        }
    }

    // Inbound: SessionRouter (priority 20 — computes session_key)
    registry.register(Arc::new(SessionRouter::new()));

    // Inbound: ContentNormalizer (priority 30)
    registry.register(Arc::new(ContentNormalizer::new()));

    // Outbound: VerbosityFilter (priority 5)
    registry.register(Arc::new(VerbosityFilter));

    // Outbound: DslParser (priority 10)
    registry.register(Arc::new(DslParser));

    // Outbound: OutboundRawLogProcessor (priority 20 — if raw_log_dir is configured)
    if let Some(ref dir) = config.raw_log_dir {
        let raw_log_config = RawLogConfig {
            enabled: true,
            dir: dir.clone(),
            retention_days: 7,
        };
        registry.register(Arc::new(OutboundRawLogProcessor::new(raw_log_config)));
    }

    registry
}

/// Register the built-in outbound middlewares on a [`Gateway`].
///
/// Every newly constructed Gateway receives:
/// - [`AuditMiddleware`] — logs every outbound message for audit.
/// - [`RateLimitMiddleware`] — session-level sliding-window throttling.
pub fn register_default_middlewares(gw: &Gateway) {
    gw.add_outbound_middleware(Arc::new(outbound_middleware::audit::AuditMiddleware));
    gw.add_outbound_middleware(Arc::new(
        outbound_middleware::rate_limit::RateLimitMiddleware::new(),
    ));
}
