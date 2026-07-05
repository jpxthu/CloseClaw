//! AdapterError — common error types for IM adapters

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
