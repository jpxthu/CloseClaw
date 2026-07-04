//! Processor chain infrastructure for message processing.
//!
//! This module provides the core types and trait for building
//! inbound/outbound processor chains:
//! - [`ProcessPhase`] — selects which chain a processor belongs to
//! - [`MessageProcessor`] — trait for message processors
//! - [`MessageContext`] — context carried through the chain
//! - [`ProcessedMessage`] — output after the chain finishes
//! - [`NormalizedMessage`] (from common) — input to the inbound chain
//! - [`RawMessageLog`] — snapshot of normalized message at each processing step
//! - [`ProcessError`] — error types

#[cfg(test)]
mod build_processor_registry_tests;
pub mod content_normalizer;
pub mod context;
#[cfg(test)]
mod context_tests;
pub mod dsl_parser;
#[cfg(test)]
mod dsl_parser_tests;
pub mod error;
pub mod loader;
pub mod middleware;
#[cfg(test)]
mod middleware_tests;
#[cfg(test)]
mod outbound_chain_tests;
pub mod outbound_raw_log;
#[cfg(test)]
mod outbound_raw_log_tests;
pub mod processor;
pub mod raw_log_processor;
pub mod registry;
#[cfg(test)]
mod registry_tests;
pub mod session_router;
pub mod verbosity_filter;
#[cfg(test)]
mod verbosity_filter_tests;

pub use dsl_parser::DslParser;
pub use loader::{ProcessorChainConfig, ProcessorChainLoader, ProcessorConfig};
pub use registry::ProcessorRegistry;
pub use session_router::SessionRouter;

pub use closeclaw_common::im_plugin::NormalizedMessage;
pub use closeclaw_common::processor::{DslInstruction, DslParseResult, ProcessedMessage};
pub use context::{MessageContext, RawMessageLog};
pub use error::ProcessError;
pub use middleware::{run_middleware_chain, MiddlewareError, OutboundMiddleware};
pub use processor::{MessageProcessor, ProcessPhase};

use std::sync::Arc;

use closeclaw_gateway::GatewayConfig;

use self::content_normalizer::ContentNormalizer;
use self::outbound_raw_log::OutboundRawLogProcessor;
use self::raw_log_processor::{RawLogConfig, RawLogProcessor};
use self::verbosity_filter::VerbosityFilter;

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
        let processor =
            RawLogProcessor::new(raw_log_config).expect("RawLogProcessor initialization failed");
        registry.register(Arc::new(processor));
    }

    // Inbound: SessionRouter (priority 20 — computes session_key)
    registry.register(Arc::new(SessionRouter::new(config.dm_scope)));

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
