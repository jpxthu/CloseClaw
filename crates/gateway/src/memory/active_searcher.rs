//! Active-searcher: background context injection for sessions.
//!
//! This module re-exports the full implementation from `closeclaw-memory`.
//! Gateway does not maintain its own stub — the canonical types and logic
//! live in the memory crate.

pub use closeclaw_memory::active_searcher::{
    ActiveSearcher, ActiveSearcherConfig, ActiveSearcherError,
};
