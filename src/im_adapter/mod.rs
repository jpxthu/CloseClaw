//! im_adapter — Core types and rendering abstractions for IM adapters
//!
//! This module unifies IMPlugin, NormalizedMessage, AdapterError,
//! and RenderedOutput under a single entry point.

pub mod code_block;
pub mod error;
pub mod normalized;
pub mod plugin;
pub mod streaming;

pub use error::AdapterError;
pub use normalized::NormalizedMessage;
pub use plugin::{IMPlugin, RenderedOutput};
