//! Feishu Stream Fallback Steps Definition
//!
//! Defines the default fallback steps for Stream→Plan mode switch on Feishu.

use serde::{Deserialize, Serialize};

/// Fallback action types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackAction {
    /// Send initial hint message
    SendInitialMessage,
    /// Create interactive card
    CreateCard,
    /// Update card content
    UpdateCard,
    /// Send final conclusion
    SendFinal,
}

/// A single fallback step
#[derive(Debug, Clone)]
pub struct FallbackStep {
    /// Step number (1-indexed)
    pub step: u32,
    /// Action type for this step
    pub action: FallbackAction,
    /// Content/description for this step (static string)
    pub content: &'static str,
    /// Whether this step should persist in history
    pub persist: bool,
}

/// Default fallback steps for Feishu Stream→Plan
pub const FEISHU_STREAM_FALLBACK_STEPS: &[FallbackStep] = &[
    FallbackStep {
        step: 1,
        action: FallbackAction::SendInitialMessage,
        content: "🔍 进入深度分析模式...",
        persist: true,
    },
    FallbackStep {
        step: 2,
        action: FallbackAction::CreateCard,
        content: "创建 Plan 卡片框架",
        persist: false,
    },
    FallbackStep {
        step: 3,
        action: FallbackAction::UpdateCard,
        content: "逐步更新卡片内容",
        persist: false,
    },
    FallbackStep {
        step: 4,
        action: FallbackAction::SendFinal,
        content: "发送最终结论",
        persist: true,
    },
];

/// Get fallback steps for a given mode
pub fn get_fallback_steps(mode: &str) -> Option<&'static [FallbackStep]> {
    match mode {
        "stream" => Some(FEISHU_STREAM_FALLBACK_STEPS),
        _ => None,
    }
}

/// Convert a FallbackStep's content to a String (for non-const contexts)
impl FallbackStep {
    /// Get content as owned String
    pub fn content_string(&self) -> String {
        self.content.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_steps_count() {
        assert_eq!(FEISHU_STREAM_FALLBACK_STEPS.len(), 4);
    }

    #[test]
    fn test_fallback_steps_sequence() {
        for (i, step) in FEISHU_STREAM_FALLBACK_STEPS.iter().enumerate() {
            assert_eq!(step.step, (i + 1) as u32);
        }
    }

    #[test]
    fn test_get_fallback_steps() {
        assert!(get_fallback_steps("stream").is_some());
        assert!(get_fallback_steps("unknown").is_none());
    }

    #[test]
    fn test_content_string() {
        let step = &FEISHU_STREAM_FALLBACK_STEPS[0];
        assert_eq!(step.content_string(), "🔍 进入深度分析模式...");
    }
}
