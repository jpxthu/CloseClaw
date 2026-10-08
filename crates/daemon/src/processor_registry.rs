//! Processor registry construction for the Gateway.
//!
//! Composition-root side: builds the standard inbound/outbound processor
//! chains from the [`GatewayConfig`]. The Gateway itself only consumes the
//! resulting `Arc<dyn ProcessorChain>` (common trait) — it never references
//! concrete processors.
//!
//! Cross-reference: the default chain is assembled in three copies that must
//! be kept in sync whenever a processor is added, removed or reordered:
//! - this file (production composition root)
//! - `crates/gateway/src/processor_registry_test_utils.rs` (gateway unit tests)
//! - `crates/cli/src/chat_slash_injection_tests.rs` (`chat_processor_chain`,
//!   the `raw_log_dir = None` case)

use std::sync::Arc;

use closeclaw_gateway::GatewayConfig;
use closeclaw_processor_chain::content_normalizer::ContentNormalizer;
use closeclaw_processor_chain::outbound_raw_log::OutboundRawLogProcessor;
use closeclaw_processor_chain::raw_log_processor::{RawLogConfig, RawLogProcessor};
use closeclaw_processor_chain::registry::ProcessorRegistry;
use closeclaw_processor_chain::session_router::SessionRouter;
use closeclaw_processor_chain::verbosity_filter::VerbosityFilter;
use closeclaw_processor_chain::DslParser;

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
            dir: Some(dir.clone()),
        };
        let processor = RawLogProcessor::new(raw_log_config);
        registry.register(Arc::new(processor));
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
            dir: Some(dir.clone()),
        };
        registry.register(Arc::new(OutboundRawLogProcessor::new(raw_log_config)));
    }

    registry
}

/// Convenience wrapper: the injected chain handed to `Gateway::new`.
pub fn build_processor_chain(
    config: &GatewayConfig,
) -> Arc<dyn closeclaw_common::processor::ProcessorChain> {
    Arc::new(build_processor_registry(config))
}
