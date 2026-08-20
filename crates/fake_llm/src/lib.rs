//! Fake LLM Server
//!
//! An independent HTTP test server that implements OpenAI and Anthropic
//! protocol endpoints for deterministic, reproducible black-box testing
//! of CloseClaw's LLM call chain.
//!
//! # Architecture
//!
//! The server is organized in four layers:
//! - **endpoints**: protocol-specific HTTP handlers (OpenAI, Anthropic, models)
//! - **protocol**: request parsing and response serialization per protocol
//! - **server**: Axum router configuration and server lifecycle
//! - **types**: protocol-agnostic shared types passed to the scenario engine

pub mod endpoints;
pub mod protocol;
pub mod scenario;
pub mod server;
pub mod types;

pub use scenario::types as scenario_types;
pub use scenario::{DecisionOutcome, ScenarioEngine, ScenarioState};
pub use types::{RequestFeatures, ScenarioDecision};
