//! Shared LLM error type.
//!
//! Moved from `closeclaw-llm` to `closeclaw-common` so that cross-layer
//! traits (e.g., [`LlmCaller`](crate::llm_caller::LlmCaller)) can reference
//! it without creating circular dependencies.

/// Errors that can occur during LLM operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum LLMError {
    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    /// Request was cancelled by the session's cancellation token
    /// (cascade stop or explicit `/stop`). Never retried.
    #[error("Request cancelled")]
    Cancelled,
}

/// Classifies an LLM error to determine retry strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Transient errors (429, 5xx, timeout) — retry with backoff.
    Transient,
    /// Auth errors (401, 403) — rotate credentials, do not retry same credentials.
    Auth,
    /// Billing errors (402, quota exhausted) — long cooldown.
    Billing,
    /// Invalid request (400, 422) — do not retry, switch model.
    InvalidRequest,
    /// Unknown errors — treat as transient with limited retries.
    Unknown,
}

impl LLMError {
    /// Classify this error to determine retry strategy.
    pub fn kind(&self) -> ErrorKind {
        use ErrorKind::*;
        match self {
            LLMError::AuthFailed(_) => Auth,
            LLMError::RateLimitExceeded => Transient,
            LLMError::InvalidRequest(_) | LLMError::ModelNotFound(_) => InvalidRequest,
            LLMError::ApiError(msg) => {
                if msg.contains("500")
                    || msg.contains("502")
                    || msg.contains("503")
                    || msg.contains("504")
                {
                    Transient
                } else if msg.contains("400") || msg.contains("422") {
                    InvalidRequest
                } else if msg.contains("401") || msg.contains("403") {
                    Auth
                } else {
                    Unknown
                }
            }
            LLMError::NetworkError(_) => Transient,
            LLMError::Cancelled => InvalidRequest,
        }
    }
}
