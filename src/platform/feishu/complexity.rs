//! High-Complexity Task Detection

use crate::session::events::ModeSwitchEvent;

/// Complexity indicator keywords that suggest a high-complexity task.
const COMPLEXITY_INDICATORS: &[&str] = &[
    "系统", "架构", "设计", "实现", "重构", "迁移",
];

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
