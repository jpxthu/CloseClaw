//! High-Complexity Task Detection

use crate::session::events::ModeSwitchEvent;

/// Complexity indicator keywords that suggest a high-complexity task.
const COMPLEXITY_INDICATORS: &[&str] = &["系统", "架构", "设计", "实现", "重构", "迁移"];

/// Determine whether a mode-switch event describes a high-complexity task.
///
/// A task is considered high-complexity when:
/// - At least 2 complexity indicator keywords appear in the parsed goal, OR
/// - The parsed goal exceeds 100 characters.
pub fn is_high_complexity(intent: &ModeSwitchEvent) -> bool {
    let goal = intent
        .user_intent
        .as_ref()
        .and_then(|u| u.parsed_goal.as_ref().map(|s| s.as_str()))
        .unwrap_or("");

    let indicator_count = COMPLEXITY_INDICATORS
        .iter()
        .filter(|k| goal.contains(*k))
        .count();

    indicator_count >= 2 || goal.len() > 100
}

/// Configuration for enhanced display of high-complexity tasks.
#[derive(Debug, Clone)]
pub struct HighComplexityConfig {
    /// Show a visual progress bar in the card.
    pub show_progress_bar: bool,
    /// Highlight key decision points.
    pub show_key_decision_points: bool,
    /// Enable mind-map export button.
    pub enable_mind_map_export: bool,
    /// Require user confirmation before each step.
    pub enable_step_confirmation: bool,
}

impl Default for HighComplexityConfig {
    fn default() -> Self {
        Self {
            show_progress_bar: true,
            show_key_decision_points: true,
            enable_mind_map_export: false,
            enable_step_confirmation: true,
        }
    }
}

/// Get the default high-complexity configuration.
pub fn get_high_complexity_config() -> HighComplexityConfig {
    HighComplexityConfig::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::events::{ModeSwitchEvent, UserIntent};
    use std::sync::Arc;

    fn make_event(goal: &str) -> ModeSwitchEvent {
        ModeSwitchEvent {
            user_intent: Some(Arc::new(UserIntent::new("test").with_parsed_goal(goal))),
            ..ModeSwitchEvent::default()
        }
    }

    #[test]
    fn test_not_high_complexity_simple() {
        let event = make_event("帮我写一个hello world");
        assert!(!is_high_complexity(&event));
    }

    #[test]
    fn test_high_complexity_two_keywords() {
        let event = make_event("系统架构重构任务");
        assert!(is_high_complexity(&event));
    }

    #[test]
    fn test_high_complexity_long_goal() {
        let long_goal = "a".repeat(101);
        let event = make_event(&long_goal);
        assert!(is_high_complexity(&event));
    }

    #[test]
    fn test_not_high_complexity_exactly_100() {
        let goal = "a".repeat(100);
        let event = make_event(&goal);
        assert!(!is_high_complexity(&event));
    }

    #[test]
    fn test_not_high_complexity_single_keyword() {
        let event = make_event("系统监控任务");
        assert!(!is_high_complexity(&event));
    }

    #[test]
    fn test_no_user_intent() {
        let event = ModeSwitchEvent::default();
        assert!(!is_high_complexity(&event));
    }

    #[test]
    fn test_no_parsed_goal() {
        let event = ModeSwitchEvent {
            user_intent: Some(Arc::new(UserIntent::new("test"))),
            ..ModeSwitchEvent::default()
        };
        assert!(!is_high_complexity(&event));
    }

    #[test]
    fn test_default_config() {
        let config = HighComplexityConfig::default();
        assert!(config.show_progress_bar);
        assert!(config.show_key_decision_points);
        assert!(!config.enable_mind_map_export);
        assert!(config.enable_step_confirmation);
    }

    #[test]
    fn test_get_config() {
        let config = get_high_complexity_config();
        assert!(config.show_progress_bar);
    }
}
