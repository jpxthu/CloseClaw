//! Test-only default processor chain.
//!
//! Mirrors the composition-root assembly
//! (`closeclaw-daemon::processor_registry`) for the
//! `raw_log_dir = None` / `Some(..)` cases, so gateway unit tests can inject
//! the same chain production installs without reaching the composition root.

use std::sync::Arc;

use closeclaw_common::processor::ProcessorChain;
use closeclaw_processor_chain::content_normalizer::ContentNormalizer;
use closeclaw_processor_chain::outbound_raw_log::OutboundRawLogProcessor;
use closeclaw_processor_chain::raw_log_processor::{RawLogConfig, RawLogProcessor};
use closeclaw_processor_chain::registry::ProcessorRegistry;
use closeclaw_processor_chain::session_router::SessionRouter;
use closeclaw_processor_chain::verbosity_filter::VerbosityFilter;
use closeclaw_processor_chain::DslParser;

use crate::GatewayConfig;

/// Build the default inbound/outbound chain for `config`.
pub(crate) fn default_registry(config: &GatewayConfig) -> Arc<dyn ProcessorChain> {
    let mut registry = ProcessorRegistry::default();

    if let Some(ref dir) = config.raw_log_dir {
        let raw_log_config = RawLogConfig {
            enabled: true,
            dir: Some(dir.clone()),
        };
        registry.register(Arc::new(RawLogProcessor::new(raw_log_config)));
    }

    registry.register(Arc::new(SessionRouter::new()));
    registry.register(Arc::new(ContentNormalizer::new()));
    registry.register(Arc::new(VerbosityFilter));
    registry.register(Arc::new(DslParser));

    if let Some(ref dir) = config.raw_log_dir {
        let raw_log_config = RawLogConfig {
            enabled: true,
            dir: Some(dir.clone()),
        };
        registry.register(Arc::new(OutboundRawLogProcessor::new(raw_log_config)));
    }

    Arc::new(registry)
}
