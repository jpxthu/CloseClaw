//! Error types for the execution engine.

use thiserror::Error;

/// Errors that can occur during execution engine operations.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ExecutionError {
    /// Sub-agent spawn failed.
    #[error("spawn failed: {message}")]
    SpawnFailed {
        /// Descriptive error message.
        message: String,
    },

    /// Sub-agent returned an invalid result.
    #[error("invalid result from sub-agent: {message}")]
    InvalidResult {
        /// Descriptive error message.
        message: String,
    },

    /// Maximum retries exceeded for a step.
    #[error("max retries ({max}) exceeded for step {step_index}")]
    MaxRetriesExceeded {
        /// The step index that failed.
        step_index: usize,
        /// The configured max retries.
        max: u32,
    },

    /// Step execution returned an error.
    #[error("step {step_index} failed: {message}")]
    StepFailed {
        /// The step index that failed.
        step_index: usize,
        /// Error message from the step.
        message: String,
    },
}
