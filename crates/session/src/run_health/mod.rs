//! Session-level run health detection.
//!
//! This module provides hard-rule health checking and failure
//! classification for individual turns within a session. It is a
//! pure-code component that does not depend on LLM calls.
//!
//! # Components
//!
//! - **Hard-rule engine** — evaluates deterministic checks (timeout,
//!   empty response, structural anomaly, retry exhaustion) and
//!   aggregates violations.
//! - **Unhealthy handler** — maps failure categories to recovery
//!   actions (retry with backoff, notify user, stop).

pub mod health_types;

pub use health_types::{
    FailureCategory, HardRuleViolation, HealthCheckInput, HealthCheckOutput, HealthStatus,
    RecoverableAction, RetryPolicy,
};

#[cfg(test)]
mod health_types_tests;
