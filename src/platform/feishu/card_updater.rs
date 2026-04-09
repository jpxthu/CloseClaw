//! Card Update Service Interface
//!
//! Defines the trait for card update operations.

use async_trait::async_trait;

use super::card::{PlanCardConfig, StepStatus};
use super::error::FeishuAdapterError;

/// Section update content
#[derive(Debug, Clone, Default)]
pub struct SectionUpdate {
    /// New title (if any)
    pub title: Option<String>,
    /// New content (if any)
    pub content: Option<String>,
    /// New status (if any)
    pub status: Option<StepStatus>,
}

/// Card handle returned after card creation
#[derive(Debug, Clone)]
pub struct CardHandle {
    /// Feishu message ID of the card
    pub message_id: String,
}

/// Card service error type
#[derive(Debug, Clone)]
pub struct CardServiceError {
    pub message: String,
}

impl std::fmt::Display for CardServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CardServiceError: {}", self.message)
    }
}

impl std::error::Error for CardServiceError {}

/// Card service trait for creating and updating Feishu cards
#[async_trait]
pub trait CardService: Send + Sync {
    /// Create an interactive card
    async fn create_card(
        &self,
        config: &PlanCardConfig,
    ) -> Result<CardHandle, FeishuAdapterError>;

    /// Update a specific section of the card
    async fn update_section(
        &self,
        card_id: &str,
        section_index: usize,
        update: SectionUpdate,
    ) -> Result<(), FeishuAdapterError>;

    /// Update card progress
    async fn update_progress(
        &self,
        card_id: &str,
        current_step: u32,
        total_steps: u32,
    ) -> Result<(), FeishuAdapterError>;

    /// Mark a step as complete
    async fn mark_step_complete(
        &self,
        card_id: &str,
        step_number: u32,
    ) -> Result<(), FeishuAdapterError>;

    /// Update entire card content
    async fn update_card(
        &self,
        card_id: &str,
        config: &PlanCardConfig,
    ) -> Result<(), FeishuAdapterError>;
}
