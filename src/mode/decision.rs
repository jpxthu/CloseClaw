//! Mode Decision Tree
//!
//! Implements the mode decision logic with priority:
//! 1. Slash command (explicit)
//! 2. Natural language (implicit, confidence > 0.8)
//! 3. Platform adaptation (fallback)

use crate::platform::capabilities::{ModeDecisionContext, PlatformCapabilityService};
use crate::session::persistence::ReasoningMode;

use super::natural_language::parse_natural_language_intent;
use super::slash_command::parse_slash_command;

// Re-export SLASH_MODE_MAP from slash_command
pub use super::slash_command::SLASH_MODE_MAP;

/// Confidence threshold for natural language trigger
pub const NL_CONFIDENCE_THRESHOLD: f32 = 0.8;

/// Mode decision tree service
#[derive(Clone)]
pub struct ModeDecisionTree {
    capability_service: PlatformCapabilityService,
}

impl ModeDecisionTree {
    /// Create a new ModeDecisionTree
    pub fn new(capability_service: PlatformCapabilityService) -> Self {
        Self { capability_service }
    }

    /// Decide the mode for a given input
    ///
    /// Priority:
    /// 1. Slash command (explicit)
    /// 2. Natural language (implicit, confidence > threshold)
    /// 3. Platform adaptation (fallback for unsupported modes)
    pub fn decide(
        &self,
        user_input: &str,
        platform: &str,
        context: &ModeDecisionContext,
    ) -> ReasoningMode {
        // 1. Check for explicit slash command
        if let Some(slash_cmd) = parse_slash_command(user_input) {
            if !slash_cmd.is_meta_command {
                return slash_cmd.target_mode;
            }
        }

        // 2. Check for natural language intent with high confidence
        let nl_intent = parse_natural_language_intent(user_input);
        if nl_intent.confidence >= NL_CONFIDENCE_THRESHOLD {
            let mode = nl_intent.mode;
            // Apply platform adaptation if needed
            return self.capability_service.get_fallback_mode(platform, mode);
        }

        // 3. Platform adaptation for requested mode
        if let Some(requested_mode) = context.requested_mode {
            if !self.capability_service.supports_mode(platform, requested_mode) {
                return self.capability_service.get_fallback_mode(platform, requested_mode);
            }
            return requested_mode;
        }

        // Default to Direct
        ReasoningMode::Direct
    }
}

/// Decide the reasoning mode for a given input
///
/// This is the main entry point for mode decision, implementing the priority:
/// 1. Slash command (explicit) → corresponding mode
/// 2. Natural language (implicit) → inferred mode if confidence > 0.8
/// 3. Platform adaptation → fallback mode if requested mode not supported
pub fn decide_mode(
    user_input: &str,
    platform: &str,
    context: &ModeDecisionContext,
    capability_service: &PlatformCapabilityService,
) -> ReasoningMode {
    let tree = ModeDecisionTree::new(capability_service.clone());
    tree.decide(user_input, platform, context)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_service() -> PlatformCapabilityService {
        PlatformCapabilityService::new()
    }

    fn make_context() -> ModeDecisionContext {
        ModeDecisionContext::new("test-session")
    }

    #[test]
    fn test_slash_command_priority() {
        let service = make_service();
        let ctx = make_context();

        // /plan should trigger Plan mode
        let mode = decide_mode("/plan 设计一个缓存系统", "feishu", &ctx, &service);
        assert_eq!(mode, ReasoningMode::Plan);

        // /code should trigger Stream mode
        let mode = decide_mode("/code 写一个排序函数", "feishu", &ctx, &service);
        assert_eq!(mode, ReasoningMode::Stream);

        // /direct should trigger Direct mode
        let mode = decide_mode("/direct", "feishu", &ctx, &service);
        assert_eq!(mode, ReasoningMode::Direct);

        // /think should trigger Hidden mode
        let mode = decide_mode("/think 分析这个方案", "feishu", &ctx, &service);
        assert_eq!(mode, ReasoningMode::Hidden);
    }

    #[test]
    fn test_natural_language_high_confidence() {
        let service = make_service();
        let ctx = make_context();

        // High confidence NL trigger
        let mode = decide_mode(
            "帮我规划一下系统架构，有什么方案吗",
            "feishu",
            &ctx,
            &service,
        );
        assert_eq!(mode, ReasoningMode::Plan);
    }

    #[test]
    fn test_platform_fallback() {
        let service = make_service();
        let ctx = ModeDecisionContext::new("test-session")
            .with_requested_mode(ReasoningMode::Stream);

        // On Feishu, Stream should fall back to Plan
        let mode = decide_mode("写代码", "feishu", &ctx, &service);
        assert_eq!(mode, ReasoningMode::Plan);

        // On Telegram, Stream should be supported
        let mode = decide_mode("写代码", "telegram", &ctx, &service);
        assert_eq!(mode, ReasoningMode::Stream);
    }

    #[test]
    fn test_default_to_direct() {
        let service = make_service();
        let ctx = make_context();

        // No clear intent → Direct
        let mode = decide_mode("你好", "feishu", &ctx, &service);
        assert_eq!(mode, ReasoningMode::Direct);
    }

    #[test]
    fn test_meta_command_not_switch_mode() {
        let service = make_service();
        let ctx = make_context();

        // /help should not switch mode (it's a meta command)
        let mode = decide_mode("/help", "feishu", &ctx, &service);
        // Meta commands don't trigger mode switch, falls through to NL or default
        // Since /help has low confidence, it defaults to Direct
        assert_eq!(mode, ReasoningMode::Direct);
    }

    #[test]
    fn test_mode_decision_tree() {
        let service = make_service();
        let tree = ModeDecisionTree::new(service.clone());

        // Test via tree interface
        let ctx = make_context();
        let mode = tree.decide("/plan 设计系统", "feishu", &ctx);
        assert_eq!(mode, ReasoningMode::Plan);
    }
}
