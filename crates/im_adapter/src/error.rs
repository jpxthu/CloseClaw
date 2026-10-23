//! AdapterError — common error types for IM adapters

use closeclaw_common::AdapterError as CommonAdapterError;

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("Invalid payload: {0}")]
    InvalidPayload(String),

    #[error("Authentication failed")]
    AuthFailed,

    #[error("Send failed: {0}")]
    SendFailed(String),

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Unsupported operation")]
    UnsupportedOperation,
}

impl From<AdapterError> for CommonAdapterError {
    fn from(e: AdapterError) -> Self {
        match e {
            AdapterError::InvalidPayload(s) => CommonAdapterError::InvalidPayload(s),
            AdapterError::AuthFailed => CommonAdapterError::AuthFailed,
            AdapterError::SendFailed(s) => CommonAdapterError::SendFailed(s),
            AdapterError::InvalidSignature => CommonAdapterError::InvalidSignature,
            AdapterError::IoError(e) => CommonAdapterError::IoError(e),
            AdapterError::UnsupportedOperation => CommonAdapterError::UnsupportedOperation,
        }
    }
}
