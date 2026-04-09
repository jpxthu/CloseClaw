//! Platform Capabilities Types and Service
//!
//! Defines platform capability matrix and detection service for
//! multi-platform IM adapter support.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

pub use crate::session::persistence::ReasoningMode;

/// Capability level enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CapabilityLevel {
    /// Fully supported
    Full,
    /// Partially supported (with limitations)
    Partial,
    /// Not supported
    None,
}

impl Default for CapabilityLevel {
    fn default() -> Self {
        CapabilityLevel::None
    }
}

/// File upload capability
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileUploadCapability {
    Full,
    Partial,
    None,
}

impl Default for FileUploadCapability {
    fn default() -> Self {
        FileUploadCapability::None
    }
}

/// Message update capability
pub type MessageUpdateCapability = CapabilityLevel;

/// Card interaction capability
pub type CardInteractionCapability = CapabilityLevel;

/// Platform Capabilities — 平台能力结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformCapabilities {
    /// Platform identifier (e.g., "feishu", "telegram")
    pub platform: String,
    /// Message update capability
    pub message_update: MessageUpdateCapability,
    /// Card interaction capability
    pub card_interaction: CardInteractionCapability,
    /// File upload capability
    pub file_upload: FileUploadCapability,
    /// Maximum message length in characters
    pub message_length_limit: u32,
    /// Stream mode support level
    pub stream_mode_support: CapabilityLevel,
    /// Whether editing sent messages is supported
    pub edit_message_support: bool,
}

/// Platform Capability Service — 平台能力检测服务
#[derive(Clone)]
pub struct PlatformCapabilityService {
    /// Platform capabilities map
    capabilities: HashMap<String, PlatformCapabilities>,
}

impl PlatformCapabilityService {
    /// Create a new PlatformCapabilityService with known platform configurations
    pub fn new() -> Self {
        let mut capabilities = HashMap::new();

        // Feishu capabilities
        capabilities.insert(
            "feishu".to_string(),
            PlatformCapabilities {
                platform: "feishu".to_string(),
                message_update: CapabilityLevel::Partial,
                card_interaction: CapabilityLevel::Full,
                file_upload: FileUploadCapability::Full,
                message_length_limit: 10000,
                stream_mode_support: CapabilityLevel::Partial, // Stream updates会被覆盖，需要降级
                edit_message_support: true,
            },
        );

        // Telegram capabilities
        capabilities.insert(
            "telegram".to_string(),
            PlatformCapabilities {
                platform: "telegram".to_string(),
                message_update: CapabilityLevel::Full,
                card_interaction: CapabilityLevel::Partial,
                file_upload: FileUploadCapability::Full,
                message_length_limit: 4096,
                stream_mode_support: CapabilityLevel::Full,
                edit_message_support: true,
            },
        );

        // Discord capabilities
        capabilities.insert(
            "discord".to_string(),
            PlatformCapabilities {
                platform: "discord".to_string(),
                message_update: CapabilityLevel::Full,
                card_interaction: CapabilityLevel::Partial,
                file_upload: FileUploadCapability::Full,
                message_length_limit: 2000,
                stream_mode_support: CapabilityLevel::Full,
                edit_message_support: true,
            },
        );

        // Slack capabilities
        capabilities.insert(
            "slack".to_string(),
            PlatformCapabilities {
                platform: "slack".to_string(),
                message_update: CapabilityLevel::Full,
                card_interaction: CapabilityLevel::Partial,
                file_upload: FileUploadCapability::Full,
                message_length_limit: 3000,
                stream_mode_support: CapabilityLevel::Full,
                edit_message_support: true,
            },
        );

        Self { capabilities }
    }

    /// Get capabilities for a specific platform
    pub fn get_capabilities(&self, platform: &str) -> PlatformCapabilities {
        self.capabilities
            .get(platform)
            .cloned()
            .unwrap_or_else(PlatformCapabilities::default)
    }

    /// Check if a platform supports a specific reasoning mode
    pub fn supports_mode(&self, platform: &str, mode: ReasoningMode) -> bool {
        let caps = self.get_capabilities(platform);
        match mode {
            ReasoningMode::Direct | ReasoningMode::Plan | ReasoningMode::Hidden => true, // 所有平台都支持
            ReasoningMode::Stream => caps.stream_mode_support != CapabilityLevel::None,
        }
    }

    /// Get the fallback mode for a platform when the requested mode is not supported
    pub fn get_fallback_mode(&self, platform: &str, requested_mode: ReasoningMode) -> ReasoningMode {
        match requested_mode {
            ReasoningMode::Stream => {
                let caps = self.get_capabilities(platform);
                if caps.stream_mode_support == CapabilityLevel::None {
                    // Stream not supported at all → fallback to Direct
                    ReasoningMode::Direct
                } else if caps.stream_mode_support == CapabilityLevel::Partial {
                    // Partial support → fallback to Plan
                    ReasoningMode::Plan
                } else {
                    ReasoningMode::Stream
                }
            }
            _ => requested_mode,
        }
    }
}

impl Default for PlatformCapabilityService {
    fn default() -> Self {
        Self::new()
    }
}

/// Context for mode decision
#[derive(Debug, Clone)]
pub struct ModeDecisionContext {
    /// Current requested mode
    pub requested_mode: Option<ReasoningMode>,
    /// Session ID
    pub session_id: String,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

impl ModeDecisionContext {
    /// Create a new ModeDecisionContext
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            requested_mode: None,
            session_id: session_id.into(),
            metadata: HashMap::new(),
        }
    }

    /// Set the requested mode
    pub fn with_requested_mode(mut self, mode: ReasoningMode) -> Self {
        self.requested_mode = Some(mode);
        self
    }

    /// Add metadata
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_level_default() {
        assert_eq!(CapabilityLevel::default(), CapabilityLevel::None);
    }

    #[test]
    fn test_platform_capabilities_equality() {
        let caps1 = PlatformCapabilities {
            platform: "feishu".to_string(),
            message_update: CapabilityLevel::Partial,
            card_interaction: CapabilityLevel::Full,
            file_upload: FileUploadCapability::Full,
            message_length_limit: 10000,
            stream_mode_support: CapabilityLevel::Partial,
            edit_message_support: true,
        };

        let caps2 = PlatformCapabilities {
            platform: "feishu".to_string(),
            message_update: CapabilityLevel::Partial,
            card_interaction: CapabilityLevel::Full,
            file_upload: FileUploadCapability::Full,
            message_length_limit: 10000,
            stream_mode_support: CapabilityLevel::Partial,
            edit_message_support: true,
        };

        assert_eq!(caps1.platform, caps2.platform);
    }

    #[test]
    fn test_mode_decision_context() {
        let ctx = ModeDecisionContext::new("session-123")
            .with_requested_mode(ReasoningMode::Stream)
            .with_metadata("user_id", "user-456");

        assert_eq!(ctx.session_id, "session-123");
        assert_eq!(ctx.requested_mode, Some(ReasoningMode::Stream));
        assert_eq!(ctx.metadata.get("user_id"), Some(&"user-456".to_string()));
    }
}
