//! Memory — long-term memory for agents.
//!
//! Provides dreaming (three-stage memory promotion) and memory-mining
//! (session transcript extraction) capabilities.

pub mod dreaming;
pub mod miner;

#[cfg(test)]
mod dreaming_tests;
#[cfg(test)]
mod miner_tests;
