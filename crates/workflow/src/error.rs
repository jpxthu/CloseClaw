//! Workflow engine error types.

/// Unified error type for workflow engine operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum WorkflowError {
    /// The workflow definition is invalid or missing required fields.
    #[error("invalid workflow definition: {0}")]
    InvalidDefinition(String),

    /// YAML frontmatter parsing failed.
    #[error("failed to parse workflow definition: {0}")]
    ParseError(String),

    /// The referenced step does not exist in the workflow.
    #[error("step not found: {0}")]
    StepNotFound(usize),

    /// No transition matched the provided answers and no default exists.
    #[error("no transitions matched")]
    NoMatchingTransition,

    /// Agent called workflow_blocked on a step where allow_blocked is false.
    #[error("blocking not allowed for this step")]
    BlockingNotAllowed,

    /// Verify retry count exceeded the configured limit.
    #[error("verify limit exceeded")]
    VerifyLimitExceeded,
}

impl WorkflowError {
    /// Wrap a serde_yaml error into a WorkflowError::ParseError.
    pub fn from_yaml_error(err: serde_yaml::Error) -> Self {
        Self::ParseError(err.to_string())
    }

    /// Create an InvalidDefinition error with a message.
    pub fn invalid_definition(msg: impl Into<String>) -> Self {
        Self::InvalidDefinition(msg.into())
    }
}
