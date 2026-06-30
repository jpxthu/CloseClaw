//! Memory — long-term memory for agents.
//!
//! Provides dreaming (three-stage memory promotion) and memory-mining
//! (session transcript extraction) capabilities.

pub mod active_searcher;
pub mod active_searcher_llm;
pub mod dreaming;
pub mod miner;

#[cfg(test)]
pub mod test_helpers;

pub use active_searcher::{ActiveSearcher, ActiveSearcherConfig};
pub use closeclaw_llm::session::{InjectionPosition, MemoryInjection};

#[cfg(test)]
mod active_searcher_tests;
#[cfg(test)]
mod dreaming_tests;
#[cfg(test)]
mod miner_tests;
