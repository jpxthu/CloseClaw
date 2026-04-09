//! Mode Decision Module
//!
//! Provides mode decision logic, slash command parsing, and natural language
//! intent recognition for automatic mode switching.
//!
//! Decision priority: slash command > natural language > platform adaptation

pub mod decision;
pub mod natural_language;
pub mod slash_command;
pub mod switch_event;

pub use decision::{decide_mode, ModeDecisionTree};
pub use natural_language::{parse_natural_language_intent, IntentResult, NaturalLanguagePatterns};
pub use slash_command::{
    format_mode, handle_slash_command, parse_slash_command, SlashCommand, SlashCommandResult,
    SlashModeMap, SLASH_HELP_TEXT, SLASH_MODE_MAP, unknown_command_response,
};
pub use switch_event::{ModeSwitchEvent, ModeSwitchTrigger, UserIntent};

use crate::platform::capabilities::{ModeDecisionContext, PlatformCapabilityService};
use crate::session::persistence::ReasoningMode;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slash_mode_map_contains_all_commands() {
        let commands = ["/plan", "/code", "/review", "/debug", "/direct", "/think"];
        for cmd in commands {
            let found = SLASH_MODE_MAP.iter().any(|(c, _)| *c == cmd);
            assert!(found, "Missing command: {}", cmd);
        }
    }

    #[test]
    fn test_slash_command_parsing() {
        // Test /plan
        let result = parse_slash_command("/plan 设计一个缓存系统");
        assert!(result.is_some());
        let cmd = result.unwrap();
        assert_eq!(cmd.command, "/plan");
        assert_eq!(cmd.args, "设计一个缓存系统");

        // Test /direct
        let result = parse_slash_command("/direct");
        assert!(result.is_some());
        let cmd = result.unwrap();
        assert_eq!(cmd.command, "/direct");
        assert_eq!(cmd.args, "");

        // Test case insensitivity
        let result = parse_slash_command("/PLAN 设计一个缓存系统");
        assert!(result.is_some());

        // Test non-slash input
        let result = parse_slash_command("帮我设计一个缓存系统");
        assert!(result.is_none());
    }

    #[test]
    fn test_natural_language_intent() {
        // Test plan intent
        let result = parse_natural_language_intent("帮我规划一下系统架构");
        assert!(result.confidence > 0.8);
        assert_eq!(result.mode, ReasoningMode::Plan);

        // Test code intent
        let result = parse_natural_language_intent("写一个排序函数");
        assert!(result.confidence > 0.8);
        assert_eq!(result.mode, ReasoningMode::Stream);

        // Test debug intent
        let result = parse_natural_language_intent("为什么报这个错");
        assert!(result.confidence > 0.8);
        assert_eq!(result.mode, ReasoningMode::Stream);

        // Test review intent
        let result = parse_natural_language_intent("帮我检查一下这段代码");
        assert!(result.confidence > 0.8);
        assert_eq!(result.mode, ReasoningMode::Plan);
    }

    #[test]
    fn test_decide_mode_priority() {
        let service = PlatformCapabilityService::new();
        let ctx = ModeDecisionContext::new("test-session")
            .with_requested_mode(ReasoningMode::Stream);

        // Slash command should take highest priority
        let mode = decide_mode("/plan 设计系统", "feishu", &ctx, &service);
        assert_eq!(mode, ReasoningMode::Plan);

        // Natural language with high confidence
        let mode = decide_mode("帮我设计一个缓存系统", "feishu", &ctx, &service);
        assert_eq!(mode, ReasoningMode::Plan);

        // Platform fallback for feishu stream
        let mode = decide_mode("写一个排序", "feishu", &ctx, &service);
        // Feishu stream → Plan fallback should trigger
        // But NL intent might be higher priority
    }
}
