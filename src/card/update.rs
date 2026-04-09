//! Card update service — handles incremental updates to existing cards.
//!
//! Provides functionality to update specific elements of a card (e.g., progress bar,
//! step content) without rebuilding the entire card.

use crate::card::elements::CardElement;
use crate::card::StepStatus;

/// Card update error types
#[derive(Debug, thiserror::Error)]
pub enum CardError {
    #[error("card not found: {0}")]
    CardNotFound(String),

    #[error("update failed: {0}")]
    UpdateFailed(String),

    #[error("invalid step index: {0}")]
    InvalidStepIndex(u32),
}

/// Step update content — which fields of a step to update.
#[derive(Debug, Clone)]
pub struct PlanStepUpdate {
    /// New title (if Some)
    pub title: Option<String>,
    /// New content (if Some)
    pub content: Option<String>,
    /// New status (if Some)
    pub status: Option<StepStatus>,
}

/// Progress update content.
#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub current: u32,
    pub total: u32,
}

/// Card update service — handles card update operations.
///
/// This is a pure data structure that describes update operations.
/// The actual HTTP call to Feishu API is done by the FeishuAdapter.
#[derive(Debug, Clone)]
pub struct CardUpdateService;

impl CardUpdateService {
    /// Create a new CardUpdateService.
    pub fn new() -> Self {
        Self
    }

    /// Build a step patch for updating a specific step.
    ///
    /// Returns JSON that can be used to patch the card's step element.
    pub fn build_step_patch(step_index: u32, update: &PlanStepUpdate) -> serde_json::Value {
        let status_icon = update
            .status
            .map(|s| match s {
                StepStatus::Pending => "⏳",
                StepStatus::Active => "🔄",
                StepStatus::Completed => "✅",
            })
            .unwrap_or("○");

        let title = update.title.as_deref().unwrap_or("步骤");
        let content = update.content.as_deref().unwrap_or("");

        let markdown_content = format!("{} **{}**\n\n{}", status_icon, title, content);

        serde_json::json!({
            "tag": "markdown",
            "content": markdown_content
        })
    }

    /// Build a progress patch for refreshing the progress bar.
    ///
    /// Returns JSON for the progress element at the top of the card.
    pub fn build_progress_patch(current: u32, total: u32) -> serde_json::Value {
        let filled = "▓".repeat(current as usize);
        let empty = "░".repeat((total - current) as usize);
        let percentage = if total > 0 {
            (current * 100 / total) as u32
        } else {
            0
        };

        let content = format!(
            "**进度**: {}{} {}%\n**步骤**: {}/{}",
            filled, empty, percentage, current, total
        );

        serde_json::json!({
            "tag": "markdown",
            "content": content
        })
    }

    /// Build a full elements patch for replacing card content.
    ///
    /// This is used when Feishu requires a full replacement of the elements array.
    pub fn build_elements_patch(elements: Vec<CardElement>) -> serde_json::Value {
        use crate::card::renderer::render_element;

        let rendered: Vec<_> = elements.iter().map(render_element).collect();
        serde_json::json!({
            "elements": rendered
        })
    }
}

impl Default for CardUpdateService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_step_patch_basic() {
        let update = PlanStepUpdate {
            title: Some("新标题".to_string()),
            content: Some("新内容".to_string()),
            status: Some(StepStatus::Active),
        };

        let patch = CardUpdateService::build_step_patch(1, &update);

        assert_eq!(patch["tag"], "markdown");
        assert!(patch["content"].as_str().unwrap().contains("🔄")); // Active icon
        assert!(patch["content"].as_str().unwrap().contains("新标题"));
        assert!(patch["content"].as_str().unwrap().contains("新内容"));
    }

    #[test]
    fn test_build_step_patch_pending() {
        let update = PlanStepUpdate {
            title: None,
            content: None,
            status: Some(StepStatus::Pending),
        };

        let patch = CardUpdateService::build_step_patch(2, &update);

        let content = patch["content"].as_str().unwrap();
        assert!(content.contains("⏳"));
        assert!(content.contains("步骤")); // default title
    }

    #[test]
    fn test_build_step_patch_completed() {
        let update = PlanStepUpdate {
            title: None,
            content: None,
            status: Some(StepStatus::Completed),
        };

        let patch = CardUpdateService::build_step_patch(3, &update);

        let content = patch["content"].as_str().unwrap();
        assert!(content.contains("✅"));
    }

    #[test]
    fn test_build_step_patch_no_status() {
        let update = PlanStepUpdate {
            title: Some("测试".to_string()),
            content: None,
            status: None,
        };

        let patch = CardUpdateService::build_step_patch(1, &update);

        let content = patch["content"].as_str().unwrap();
        assert!(content.contains("○")); // default icon
        assert!(content.contains("测试"));
    }

    #[test]
    fn test_build_progress_patch() {
        let patch = CardUpdateService::build_progress_patch(2, 5);

        let content = patch["content"].as_str().unwrap();
        assert!(content.contains("▓▓░░░"));
        assert!(content.contains("40%"));
        assert!(content.contains("2/5"));
    }

    #[test]
    fn test_build_progress_patch_full() {
        let patch = CardUpdateService::build_progress_patch(5, 5);

        let content = patch["content"].as_str().unwrap();
        assert!(content.contains("▓▓▓▓▓"));
        assert!(content.contains("100%"));
        assert!(content.contains("5/5"));
    }

    #[test]
    fn test_build_progress_patch_empty() {
        let patch = CardUpdateService::build_progress_patch(0, 5);

        let content = patch["content"].as_str().unwrap();
        assert!(content.contains("░░░░░"));
        assert!(content.contains("0%"));
        assert!(content.contains("0/5"));
    }
}
