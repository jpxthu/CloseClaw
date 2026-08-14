//! Execution-specific types — re-exported from common crate.
//!
//! `ExecutionStep`, `ExecutionStepStatus`, and `TransitionError` are defined
//! in `closeclaw-common::execution_types` (shared data types) and re-exported
//! here for backward compatibility.

pub use closeclaw_common::{ExecutionStep, ExecutionStepStatus, TransitionError};
