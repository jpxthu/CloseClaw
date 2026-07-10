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

pub mod hard_rules;
pub mod health_types;
pub mod snapshot_manager;
pub mod unhealthy_handler;

pub use hard_rules::{
    EmptyResponseRule, HardRule, HardRuleEngine, RetryExhaustedRule, StructuralAnomalyRule,
    TimeoutRule,
};
pub use health_types::{
    FailureCategory, HardRuleViolation, HealthCheckInput, HealthCheckOutput, HealthStatus,
    RecoverableAction, RetryPolicy,
};
pub use snapshot_manager::{RuntimeSnapshotManager, Snapshot, TranscriptOp};
pub use unhealthy_handler::{BackoffCounter, UnhealthyHandler};

#[cfg(test)]
mod hard_rules_tests;
#[cfg(test)]
mod health_types_tests;
#[cfg(test)]
mod snapshot_manager_tests;
#[cfg(test)]
mod unhealthy_handler_tests;
