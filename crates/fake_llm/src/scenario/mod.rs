//! Scenario engine — core decision-making for Fake LLM Server.
//!
//! This module implements the scenario matching, session tracking, and
//! response generation pipeline. See `docs/design/fake_llm/scenario-engine.md`.

pub mod loader;
pub mod types;

pub use types::*;
