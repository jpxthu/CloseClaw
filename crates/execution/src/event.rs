//! Events emitted during execution engine lifecycle.

use serde::{Deserialize, Serialize};

/// Events emitted by the execution engine as steps progress.
///
/// Used by the engine to notify callers (e.g., ProgressTool, UI) of
/// step lifecycle changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionEvent {
    /// A step has started executing.
    StepStarted {
        /// Index of the step that started.
        step_index: usize,
    },
    /// A step completed successfully.
    StepCompleted {
        /// Index of the completed step.
        step_index: usize,
        /// Human-readable summary.
        summary: String,
    },
    /// A step failed.
    StepFailed {
        /// Index of the failed step.
        step_index: usize,
        /// Error message.
        error_message: String,
    },
    /// All steps have completed.
    AllCompleted,
    /// A retry has been triggered for a failed step.
    RetryTriggered {
        /// Index of the step being retried.
        step_index: usize,
        /// Current attempt number (1-based).
        attempt: u32,
    },
}
