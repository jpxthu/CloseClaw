//! Processor chain infrastructure for message processing.
//!
//! This module provides the core types and trait for building
//! inbound/outbound processor chains:
//! - [`ProcessPhase`] — selects which chain a processor belongs to
//! - [`MessageProcessor`] — trait for message processors
//! - [`MessageContext`] — context carried through the chain
//! - [`ProcessedMessage`] — output after the chain finishes
//! - [`RawMessage`] — input to the inbound chain
//! - [`RawMessageLog`] — snapshot of raw message at each processing step
//! - [`ProcessError`] — error types

pub mod context;
pub mod error;
pub mod message_cleaner;
pub mod processor;
pub mod raw_log_processor;
pub mod registry;
pub mod registry_tests;

pub use registry::ProcessorRegistry;

pub use context::{MessageContext, ProcessedMessage, RawMessage, RawMessageLog};
pub use error::ProcessError;
pub use processor::{MessageProcessor, ProcessPhase};
