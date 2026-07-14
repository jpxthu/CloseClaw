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

pub mod checker;
pub mod hard_rules;
pub mod health_types;
pub mod hook_reviewer;
pub mod llm_caller_hook_provider;
pub mod snapshot_manager;
pub mod unhealthy_handler;

pub use checker::{RunHealthChecker, RunHealthVerdict};
pub use hard_rules::{
    EmptyResponseRule, HardRule, HardRuleEngine, RetryExhaustedRule, StructuralAnomalyRule,
    TimeoutRule,
};
pub use health_types::{
    FailureCategory, HardRuleViolation, HealthCheckInput, HealthCheckOutput, HealthStatus,
    HookContext, HookToolCallInfo, RecoverableAction, RetryPolicy,
};
pub use hook_reviewer::{
    build_review_prompt, hook_prompt_template, HookConfig, HookLlmProvider, HookParams,
    HookReviewer, HookType, HookVerdict,
};
pub use llm_caller_hook_provider::LlmCallerHookProvider;
pub use snapshot_manager::{
    PersistenceMetaStore, RollbackAction, RuntimeSnapshotManager, Snapshot, SnapshotKind,
    SnapshotMeta, SnapshotMetaStore, SnapshotStatus, TranscriptOp,
};
pub use unhealthy_handler::{BackoffCounter, UnhealthyHandler};

#[cfg(test)]
mod checker_tests;
#[cfg(test)]
mod hard_rules_tests;
#[cfg(test)]
mod health_types_tests;
#[cfg(test)]
mod hook_reviewer_tests;
#[cfg(test)]
mod llm_caller_hook_provider_tests;
#[cfg(test)]
mod snapshot_manager_tests;
#[cfg(test)]
mod unhealthy_handler_tests;
