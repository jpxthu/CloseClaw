//! Mode Switch Event Definitions
//!
//! Defines the ModeSwitchEvent structure and related types for
//! publishing mode transition events to the event bus.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use crate::session::persistence::ReasoningMode;

/// Mode switch trigger type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeSwitchTrigger {
    /// Explicit slash command (e.g., /plan, /code)
    SlashCommand,
    /// Implicit natural language trigger
    NaturalLanguage,
    /// Automatic mode adaptation
    Auto,
    /// Explicit user request
    UserRequest,
}

/// User intent attached to mode switch
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserIntent {
    /// Raw user input
    pub raw_input: String,
    /// Parsed goal/task description
    pub parsed_goal: String,
}

/// Mode Switch Event — 模式切换事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeSwitchEvent {
    /// Event type identifier
    pub event_type: String,
    /// Session ID
    pub session_id: String,
    /// Event timestamp
    pub timestamp: DateTime<Utc>,
    /// Source mode
    pub from_mode: ReasoningMode,
    /// Target mode
    pub to_mode: ReasoningMode,
    /// What triggered this switch
    pub trigger: ModeSwitchTrigger,
    /// Trigger value (e.g., the slash command or NL keyword)
    pub trigger_value: String,
    /// Optional user intent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_intent: Option<UserIntent>,
}

impl ModeSwitchEvent {
    /// Create a new mode switch event
    pub fn new(
        session_id: impl Into<String>,
        from_mode: ReasoningMode,
        to_mode: ReasoningMode,
        trigger: ModeSwitchTrigger,
        trigger_value: impl Into<String>,
    ) -> Self {
        Self {
            event_type: "mode_switch".to_string(),
            session_id: session_id.into(),
            timestamp: Utc::now(),
            from_mode,
            to_mode,
            trigger,
            trigger_value: trigger_value.into(),
            user_intent: None,
        }
    }

    /// Set user intent
    pub fn with_user_intent(mut self, intent: UserIntent) -> Self {
        self.user_intent = Some(intent);
        self
    }

    /// Create from slash command
    pub fn from_slash(
        session_id: impl Into<String>,
        from_mode: ReasoningMode,
        to_mode: ReasoningMode,
        command: impl Into<String>,
        raw_input: impl Into<String>,
        args: impl Into<String>,
    ) -> Self {
        let raw: String = raw_input.into();
        Self::new(
            session_id,
            from_mode,
            to_mode,
            ModeSwitchTrigger::SlashCommand,
            command,
        )
        .with_user_intent(UserIntent {
            raw_input: raw.clone(),
            parsed_goal: args.into(),
        })
    }

    /// Create from natural language
    pub fn from_natural_language(
        session_id: impl Into<String>,
        from_mode: ReasoningMode,
        to_mode: ReasoningMode,
        keyword: impl Into<String>,
        raw_input: impl Into<String>,
    ) -> Self {
        let raw: String = raw_input.into();
        Self::new(
            session_id,
            from_mode,
            to_mode,
            ModeSwitchTrigger::NaturalLanguage,
            keyword,
        )
        .with_user_intent(UserIntent {
            raw_input: raw.clone(),
            parsed_goal: raw,
        })
    }

    /// Create from platform adaptation
    pub fn from_platform_adaptation(
        session_id: impl Into<String>,
        from_mode: ReasoningMode,
        to_mode: ReasoningMode,
        platform: impl Into<String>,
    ) -> Self {
        Self::new(
            session_id,
            from_mode,
            to_mode,
            ModeSwitchTrigger::Auto,
            format!("platform_fallback:{}", platform.into()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_switch_event_creation() {
        let event = ModeSwitchEvent::new(
            "session-123",
            ReasoningMode::Direct,
            ReasoningMode::Plan,
            ModeSwitchTrigger::SlashCommand,
            "/plan",
        );

        assert_eq!(event.event_type, "mode_switch");
        assert_eq!(event.session_id, "session-123");
        assert_eq!(event.from_mode, ReasoningMode::Direct);
        assert_eq!(event.to_mode, ReasoningMode::Plan);
        assert_eq!(event.trigger, ModeSwitchTrigger::SlashCommand);
        assert_eq!(event.trigger_value, "/plan");
    }

    #[test]
    fn test_mode_switch_event_with_user_intent() {
        let event = ModeSwitchEvent::from_slash(
            "session-123",
            ReasoningMode::Direct,
            ReasoningMode::Plan,
            "/plan",
            "/plan 设计一个缓存系统",
            "设计一个缓存系统",
        );

        assert!(event.user_intent.is_some());
        let intent = event.user_intent.unwrap();
        assert_eq!(intent.raw_input, "/plan 设计一个缓存系统");
        assert_eq!(intent.parsed_goal, "设计一个缓存系统");
    }

    #[test]
    fn test_mode_switch_trigger_variants() {
        assert_eq!(
            serde_json::to_string(&ModeSwitchTrigger::SlashCommand).unwrap(),
            "\"slash_command\""
        );
        assert_eq!(
            serde_json::to_string(&ModeSwitchTrigger::NaturalLanguage).unwrap(),
            "\"natural_language\""
        );
        assert_eq!(
            serde_json::to_string(&ModeSwitchTrigger::Auto).unwrap(),
            "\"auto\""
        );
        assert_eq!(
            serde_json::to_string(&ModeSwitchTrigger::UserRequest).unwrap(),
            "\"user_request\""
        );
    }
}
