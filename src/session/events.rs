//! Checkpoint trigger events and mode switch events
//!
//! Defines when checkpoints should be saved during the session lifecycle,
//! and the ModeSwitchEvent for handling reasoning mode transitions.

use crate::session::persistence::ReasoningMode;
use std::sync::Arc;

/// User Intent — parsed user input for reasoning
#[derive(Debug, Clone)]
pub struct UserIntent {
    /// Raw user input
    pub raw_input: String,
    /// Parsed goal (if available)
    pub parsed_goal: Option<String>,
    /// Extracted entities
    pub entities: Vec<String>,
}

impl UserIntent {
    /// Create a new UserIntent
    pub fn new(raw_input: impl Into<String>) -> Self {
        Self {
            raw_input: raw_input.into(),
            parsed_goal: None,
            entities: vec![],
        }
    }

    /// Set the parsed goal
    pub fn with_parsed_goal(mut self, goal: impl Into<String>) -> Self {
        self.parsed_goal = Some(goal.into());
        self
    }
}

impl Default for UserIntent {
    fn default() -> Self {
        Self {
            raw_input: String::new(),
            parsed_goal: None,
            entities: vec![],
        }
    }
}

/// Mode Switch Event — triggered when reasoning mode changes
#[derive(Debug, Clone)]
pub struct ModeSwitchEvent {
    /// Requested mode (from user or config)
    pub requested_mode: Option<ReasoningMode>,
    /// Target mode after switch
    pub target_mode: Option<ReasoningMode>,
    /// User intent associated with this mode switch
    pub user_intent: Option<Arc<UserIntent>>,
    /// Session ID
    pub session_id: Option<String>,
}

impl Default for ModeSwitchEvent {
    fn default() -> Self {
        Self {
            requested_mode: None,
            target_mode: None,
            user_intent: None,
            session_id: None,
        }
    }
}

impl ModeSwitchEvent {
    /// Create a new ModeSwitchEvent
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the requested mode
    pub fn with_requested_mode(mut self, mode: ReasoningMode) -> Self {
        self.requested_mode = Some(mode);
        self
    }

    /// Set the target mode
    pub fn with_target_mode(mut self, mode: ReasoningMode) -> Self {
        self.target_mode = Some(mode);
        self
    }

    /// Set the user intent
    pub fn with_user_intent(mut self, intent: UserIntent) -> Self {
        self.user_intent = Some(Arc::new(intent));
        self
    }

    /// Set the session ID
    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }
}

/// Checkpoint trigger时机 — defines when a checkpoint should be saved
#[derive(Debug, Clone)]
pub enum CheckpointTrigger {
    /// 模式切换时
    ModeSwitch {
        from_mode: ReasoningMode,
        to_mode: ReasoningMode,
    },
    /// 消息发送后
    MessageSent { message_id: String },
    /// 网关关闭前（同步写入）
    GatewayShutdown,
    /// Compaction 发生前（用于保护 bootstrap 上下文）
    PreCompact {
        /// Compaction 前 transcript 字符数
        before_char_count: usize,
    },
    /// Compaction 发生后（用于检测 bootstrap 上下文是否被扭曲）
    PostCompact {
        /// Compaction 后 transcript 字符数
        after_char_count: usize,
        /// Compaction 前 transcript 字符数
        before_char_count: usize,
    },
}

impl CheckpointTrigger {
    /// 是否需要同步写入
    pub fn requires_sync(&self) -> bool {
        matches!(self, CheckpointTrigger::GatewayShutdown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_intent_new() {
        let intent = UserIntent::new("hello world");
        assert_eq!(intent.raw_input, "hello world");
        assert!(intent.parsed_goal.is_none());
        assert!(intent.entities.is_empty());
    }

    #[test]
    fn test_user_intent_with_parsed_goal() {
        let intent = UserIntent::new("search for X").with_parsed_goal("find X");
        assert_eq!(intent.raw_input, "search for X");
        assert_eq!(intent.parsed_goal.as_deref(), Some("find X"));
    }

    #[test]
    fn test_user_intent_default() {
        let intent = UserIntent::default();
        assert!(intent.raw_input.is_empty());
        assert!(intent.parsed_goal.is_none());
        assert!(intent.entities.is_empty());
    }

    #[test]
    fn test_mode_switch_event_new_and_default() {
        let evt = ModeSwitchEvent::new();
        assert!(evt.requested_mode.is_none());
        assert!(evt.target_mode.is_none());
        assert!(evt.user_intent.is_none());
        assert!(evt.session_id.is_none());

        let evt2 = ModeSwitchEvent::default();
        assert!(evt2.requested_mode.is_none());
    }

    #[test]
    fn test_mode_switch_event_builders() {
        let intent = UserIntent::new("analyze this");
        let evt = ModeSwitchEvent::new()
            .with_requested_mode(ReasoningMode::Direct)
            .with_target_mode(ReasoningMode::Plan)
            .with_user_intent(intent)
            .with_session_id("sess-123");

        assert_eq!(evt.requested_mode, Some(ReasoningMode::Direct));
        assert_eq!(evt.target_mode, Some(ReasoningMode::Plan));
        assert!(evt.user_intent.is_some());
        assert_eq!(evt.session_id.as_deref(), Some("sess-123"));
    }

    #[test]
    fn test_checkpoint_trigger_variants_debug() {
        let mode_switch = CheckpointTrigger::ModeSwitch {
            from_mode: ReasoningMode::Direct,
            to_mode: ReasoningMode::Plan,
        };
        let debug = format!("{:?}", mode_switch);
        assert!(debug.contains("ModeSwitch"));

        let msg = CheckpointTrigger::MessageSent {
            message_id: "m1".to_string(),
        };
        let debug = format!("{:?}", msg);
        assert!(debug.contains("MessageSent"));

        let pre = CheckpointTrigger::PreCompact {
            before_char_count: 42,
        };
        let debug = format!("{:?}", pre);
        assert!(debug.contains("PreCompact"));
    }

    #[test]
    fn test_trigger_requires_sync() {
        let shutdown = CheckpointTrigger::GatewayShutdown;
        assert!(shutdown.requires_sync());

        let mode_switch = CheckpointTrigger::ModeSwitch {
            from_mode: ReasoningMode::Direct,
            to_mode: ReasoningMode::Plan,
        };
        assert!(!mode_switch.requires_sync());

        let message_sent = CheckpointTrigger::MessageSent {
            message_id: "msg123".to_string(),
        };
        assert!(!message_sent.requires_sync());

        let pre_compact = CheckpointTrigger::PreCompact {
            before_char_count: 1000,
        };
        assert!(!pre_compact.requires_sync());

        let post_compact = CheckpointTrigger::PostCompact {
            before_char_count: 1000,
            after_char_count: 500,
        };
        assert!(!post_compact.requires_sync());
    }
}
