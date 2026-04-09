//! Platform Capability Detection Module
//!
//! Provides unified platform capability detection and reasoning mode fallback
//! for various IM platforms (Feishu, Telegram, Discord, Slack).
//!
//! This module is the foundation for downstream features:
//! - #161: Feishu Stream→Plan fallback
//! - #162: Slash command system
//! - #163: Card interaction system

pub mod capabilities;
pub mod feishu;

pub use capabilities::{
    CapabilityLevel, FileUploadCapability, MessageUpdateCapability, ModeDecisionContext,
    PlatformCapabilities, PlatformCapabilityService, ReasoningMode,
};

use std::collections::HashMap;
use std::sync::Arc;

/// Known platform identifiers
pub const PLATFORM_FEISHU: &str = "feishu";
pub const PLATFORM_TELEGRAM: &str = "telegram";
pub const PLATFORM_DISCORD: &str = "discord";
pub const PLATFORM_SLACK: &str = "slack";

/// Default capabilities for unknown platforms (lazy initialization)
pub fn default_capabilities() -> PlatformCapabilities {
    PlatformCapabilities {
        platform: "unknown".to_string(),
        message_update: CapabilityLevel::None,
        card_interaction: CapabilityLevel::None,
        file_upload: FileUploadCapability::None,
        message_length_limit: 4000,
        stream_mode_support: CapabilityLevel::None,
        edit_message_support: false,
    }
}

impl Default for PlatformCapabilities {
    fn default() -> Self {
        default_capabilities()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feishu_stream_fallback() {
        let service = PlatformCapabilityService::new();
        let caps = service.get_capabilities(PLATFORM_FEISHU);

        // Feishu should support partial stream (can be downgraded)
        assert_eq!(caps.stream_mode_support, CapabilityLevel::Partial);

        // Stream on Feishu should fall back to Plan
        let fallback = service.get_fallback_mode(PLATFORM_FEISHU, ReasoningMode::Stream);
        assert_eq!(fallback, ReasoningMode::Plan);
    }

    #[test]
    fn test_telegram_full_capabilities() {
        let service = PlatformCapabilityService::new();
        let caps = service.get_capabilities(PLATFORM_TELEGRAM);

        // Telegram supports full stream
        assert!(service.supports_mode(PLATFORM_TELEGRAM, ReasoningMode::Stream));
        assert!(service.supports_mode(PLATFORM_TELEGRAM, ReasoningMode::Direct));
        assert!(service.supports_mode(PLATFORM_TELEGRAM, ReasoningMode::Plan));
    }

    #[test]
    fn test_all_platforms_have_caps() {
        let service = PlatformCapabilityService::new();
        let platforms = [PLATFORM_FEISHU, PLATFORM_TELEGRAM, PLATFORM_DISCORD, PLATFORM_SLACK];

        for platform in platforms {
            let caps = service.get_capabilities(platform);
            assert_eq!(caps.platform, platform);
        }
    }

    #[test]
    fn test_unknown_platform_defaults() {
        let service = PlatformCapabilityService::new();
        let caps = service.get_capabilities("unknown_platform");
        assert_eq!(caps.platform, "unknown");
        assert_eq!(caps.message_update, CapabilityLevel::None);
    }
}
