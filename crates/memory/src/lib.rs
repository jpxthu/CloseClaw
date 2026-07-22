//! Memory — long-term memory for agents.
//!
//! Provides dreaming (three-stage memory promotion) and memory-mining
//! (session transcript extraction) capabilities.

pub mod active_searcher;
pub mod active_searcher_llm;
pub mod dreaming;
pub mod dreaming_llm;
pub mod embedding;
pub mod miner;
pub mod miner_llm;
pub mod miner_transcript;

#[cfg(test)]
pub mod test_helpers;

pub use active_searcher::{ActiveSearcher, ActiveSearcherConfig};
pub use closeclaw_session::llm_session::{InjectionPosition, MemoryInjection};
pub use embedding::{cosine_similarity, EntityEmbedder, NgramEmbedder};

#[cfg(test)]
mod active_searcher_tests;
#[cfg(test)]
mod dreaming_gap_fix_tests;
#[cfg(test)]
mod dreaming_scoring_tests;
#[cfg(test)]
mod dreaming_status_tests;
#[cfg(test)]
mod dreaming_tests;
#[cfg(test)]
mod miner_tests;
