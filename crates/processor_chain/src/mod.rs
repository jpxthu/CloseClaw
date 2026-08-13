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

pub mod content_normalizer;
pub mod context;
#[cfg(test)]
mod context_tests;
pub mod dsl_parser;
#[cfg(test)]
mod dsl_parser_tests;
pub mod error;
pub mod loader;
#[cfg(test)]
mod loader_tests;
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
#[cfg(test)]
mod streaming_incremental_tests;
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
pub use middleware::{
    run_middleware_chain, run_pre_flight_check, MiddlewareError, OutboundMiddleware,
};
pub use processor::{MessageProcessor, ProcessPhase};

// Re-export types used by test files via `use super::*;`
#[cfg(test)]
use self::raw_log_processor::RawLogConfig;
