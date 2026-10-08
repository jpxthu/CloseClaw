//! Memory subsystem for the gateway.
//!
//! Provides active-searcher functionality that runs in the background
//! to inject relevant context into sessions.

pub mod active_searcher;
pub mod active_searcher_llm;
pub mod injection_convert;

#[cfg(test)]
mod active_searcher_llm_tests;
#[cfg(test)]
mod injection_convert_tests;
