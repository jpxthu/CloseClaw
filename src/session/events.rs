//! Checkpoint trigger events
//!
//! Defines when checkpoints should be saved during the session lifecycle.

use crate::session::persistence::ReasoningMode;

/// Checkpoint trigger时机 — defines when a checkpoint should be saved
#[derive(Debug, Clone)]
pub enum CheckpointTrigger {
    /// 模式切换时
    ModeSwitch {
        from_mode: ReasoningMode,
        to_mode: ReasoningMode,
    },
    /// 消息发送后
    MessageSent {
        message_id: String,
    },
    /// 网关关闭前（同步写入）
    GatewayShutdown,
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
    }
}
