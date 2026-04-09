//! Feishu Adapter Error Types

use thiserror::Error;

/// FeishuAdapter errors
#[derive(Error, Debug)]
pub enum FeishuAdapterError {
    #[error("Card service error: {0}")]
    CardService(String),

    #[error("Message service error: {0}")]
    MessageService(String),

    #[error("Capability service error: {0}")]
    CapabilityService(String),

    #[error("Fallback not enabled")]
    FallbackNotEnabled,

    #[error("Card not found: {0}")]
    CardNotFound(String),

    #[error("Invalid mode switch event")]
    InvalidModeSwitchEvent,

    #[error("Section not found: index {0}")]
    SectionNotFound(usize),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
