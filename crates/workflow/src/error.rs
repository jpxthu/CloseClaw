//! Workflow engine error types.

use std::fmt;

/// Unified error type for workflow engine operations.
#[derive(Debug, Clone)]
pub enum WorkflowError {
    /// The workflow definition is invalid or missing required fields.
    InvalidDefinition(String),
    /// The referenced step does not exist in the workflow.
    StepNotFound(usize),
    /// No transition matched the provided answers and no default exists.
    NoMatchingTransition,
    /// Agent called workflow_blocked on a step where allow_blocked is false.
    BlockingNotAllowed,
    /// Verify retry count exceeded the configured limit.
    VerifyLimitExceeded,
}

impl fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDefinition(msg) => write!(f, "invalid workflow definition: {msg}"),
            Self::StepNotFound(id) => write!(f, "step not found: {id}"),
            Self::NoMatchingTransition => write!(f, "no transitions matched"),
            Self::BlockingNotAllowed => write!(f, "blocking not allowed for this step"),
            Self::VerifyLimitExceeded => write!(f, "verify limit exceeded"),
        }
    }
}

impl std::error::Error for WorkflowError {}
