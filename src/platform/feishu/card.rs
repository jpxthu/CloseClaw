//! Plan Card Structure for Feishu
//!
//! Defines the card structure used for Plan mode display on Feishu.

use serde::{Deserialize, Serialize};

/// Plan card configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanCardConfig {
    /// Card title
    pub title: String,
    /// Card sections
    pub sections: Vec<PlanSection>,
    /// Whether to show progress bar
    pub show_progress: bool,
    /// Whether to show step buttons
    pub show_step_buttons: bool,
}

/// A single section within the Plan card
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanSection {
    /// Step number
    pub step_number: u32,
    /// Section title
    pub title: String,
    /// Section content
    pub content: String,
    /// Current status
    pub status: StepStatus,
}

/// Step status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    /// Not yet started
    Pending,
    /// Currently being processed
    Active,
    /// Completed
    Completed,
}

impl Default for StepStatus {
    fn default() -> Self {
        StepStatus::Pending
    }
}

/// Build initial sections for a given goal
pub fn build_initial_sections(goal: Option<&str>) -> Vec<PlanSection> {
    vec![
        PlanSection {
            step_number: 1,
            title: "需求分析".to_string(),
            content: goal
                .map(|g| format!("目标：{}", g))
                .unwrap_or_else(|| "分析中...".to_string()),
            status: StepStatus::Pending,
        },
        PlanSection {
            step_number: 2,
            title: "技术方案".to_string(),
            content: "待确定...".to_string(),
            status: StepStatus::Pending,
        },
        PlanSection {
            step_number: 3,
            title: "实现路径".to_string(),
            content: "待确定...".to_string(),
            status: StepStatus::Pending,
        },
    ]
}

/// Build a default Plan card configuration
pub fn default_plan_card_config(goal: Option<&str>) -> PlanCardConfig {
    PlanCardConfig {
        title: "深度分析计划".to_string(),
        sections: build_initial_sections(goal),
        show_progress: true,
        show_step_buttons: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_initial_sections() {
        let sections = build_initial_sections(Some("实现用户登录功能"));
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].title, "需求分析");
        assert!(sections[0].content.contains("实现用户登录功能"));
    }

    #[test]
    fn test_step_status_default() {
        assert_eq!(StepStatus::default(), StepStatus::Pending);
    }

    #[test]
    fn test_default_plan_card_config() {
        let config = default_plan_card_config(None);
        assert_eq!(config.title, "深度分析计划");
        assert_eq!(config.sections.len(), 3);
        assert!(config.show_progress);
        assert!(config.show_step_buttons);
    }
}
