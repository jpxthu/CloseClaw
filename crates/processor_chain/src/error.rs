//! Error types for the processor chain.

use thiserror::Error;

/// Errors raised during message processing.
#[derive(Debug, Error)]
pub enum ProcessError {
    /// A processor in the chain returned an error.
    #[error("processor `{name}` failed")]
    ProcessorFailed {
        /// Processor name.
        name: String,
        /// Underlying error.
        #[source]
        source: anyhow::Error,
    },

    /// The inbound/outbound message was malformed.
    #[error("invalid message: {0}")]
    InvalidMessage(String),

    /// The processor chain itself failed (e.g., empty chain misconfiguration).
    #[error("chain failed: {0}")]
    ChainFailed(String),
}

impl ProcessError {
    /// Constructs a `ProcessorFailed` error from a processor name and message.
    #[inline]
    pub fn processor_failed(name: impl Into<String>, source: impl std::fmt::Display) -> Self {
        Self::ProcessorFailed {
            name: name.into(),
            source: anyhow::Error::msg(source.to_string()),
        }
    }

    /// Constructs an `InvalidMessage` error.
    #[inline]
    pub fn invalid_message(msg: impl Into<String>) -> Self {
        Self::InvalidMessage(msg.into())
    }

    /// Constructs a `ChainFailed` error.
    #[inline]
    pub fn chain_failed(msg: impl Into<String>) -> Self {
        Self::ChainFailed(msg.into())
    }
}
