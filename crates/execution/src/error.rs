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

    /// Step execution returned an error.
    #[error("step {step_index} failed: {message}")]
    StepFailed {
        /// The step index that failed.
        step_index: usize,
        /// Error message from the step.
        message: String,
    },

    /// Permission check denied the step execution.
    #[error("permission denied for step {step_index}: {reason}")]
    PermissionDenied {
        /// The step index that was denied.
        step_index: usize,
        /// Reason for the denial.
        reason: String,
    },

    /// Step selection index out of bounds.
    #[error("step index {index} out of range (total {total})")]
    InvalidStepSelection {
        /// The invalid index.
        index: usize,
        /// Total number of available steps.
        total: usize,
    },
}
