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
    async fn create_card(&self, config: &PlanCardConfig) -> Result<CardHandle, FeishuAdapterError>;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_section_update_default() {
        let update = SectionUpdate::default();
        assert!(update.title.is_none());
        assert!(update.content.is_none());
        assert!(update.status.is_none());
    }

    #[test]
    fn test_section_update_with_fields() {
        let update = SectionUpdate {
            title: Some("Title".to_string()),
            content: Some("Content".to_string()),
            status: Some(StepStatus::Completed),
        };
        assert_eq!(update.title.as_deref(), Some("Title"));
        assert_eq!(update.content.as_deref(), Some("Content"));
    }

    #[test]
    fn test_card_handle() {
        let handle = CardHandle {
            message_id: "om_123".to_string(),
        };
        assert_eq!(handle.message_id, "om_123");
    }

    #[test]
    fn test_card_service_error_display() {
        let err = CardServiceError {
            message: "something failed".to_string(),
        };
        assert_eq!(format!("{}", err), "CardServiceError: something failed");
    }

    #[test]
    fn test_card_service_error_is_error() {
        let err = CardServiceError {
            message: "test".to_string(),
        };
        let _: &dyn std::error::Error = &err;
    }

    /// A mock CardService for testing the trait interface
    struct MockCardService;

    #[async_trait]
    impl CardService for MockCardService {
        async fn create_card(
            &self,
            _config: &PlanCardConfig,
        ) -> Result<CardHandle, FeishuAdapterError> {
            Ok(CardHandle {
                message_id: "om_mock".to_string(),
            })
        }

        async fn update_section(
            &self,
            card_id: &str,
            _section_index: usize,
            _update: SectionUpdate,
        ) -> Result<(), FeishuAdapterError> {
            if card_id == "fail" {
                return Err(FeishuAdapterError::CardService("update failed".to_string()));
            }
            Ok(())
        }

        async fn update_progress(
            &self,
            _card_id: &str,
            _current_step: u32,
            _total_steps: u32,
        ) -> Result<(), FeishuAdapterError> {
            Ok(())
        }

        async fn mark_step_complete(
            &self,
            _card_id: &str,
            _step_number: u32,
        ) -> Result<(), FeishuAdapterError> {
            Ok(())
        }

        async fn update_card(
            &self,
            _card_id: &str,
            _config: &PlanCardConfig,
        ) -> Result<(), FeishuAdapterError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_mock_card_service_create() {
        let service = MockCardService;
        let config = PlanCardConfig {
            title: "Test".to_string(),
            sections: vec![],
            show_progress: false,
            show_step_buttons: false,
        };
        let handle = service.create_card(&config).await.unwrap();
        assert_eq!(handle.message_id, "om_mock");
    }

    #[tokio::test]
    async fn test_mock_card_service_update_section() {
        let service = MockCardService;
        let update = SectionUpdate {
            title: Some("Updated".to_string()),
            content: None,
            status: None,
        };
        assert!(service.update_section("ok", 0, update).await.is_ok());
        assert!(service
            .update_section("fail", 0, SectionUpdate::default())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn test_mock_card_service_update_progress() {
        let service = MockCardService;
        assert!(service.update_progress("id", 1, 3).await.is_ok());
    }

    #[tokio::test]
    async fn test_mock_card_service_mark_step() {
        let service = MockCardService;
        assert!(service.mark_step_complete("id", 1).await.is_ok());
    }

    #[tokio::test]
    async fn test_mock_card_service_update_card() {
        let service = MockCardService;
        let config = PlanCardConfig {
            title: "Test".to_string(),
            sections: vec![],
            show_progress: false,
            show_step_buttons: false,
        };
        assert!(service.update_card("id", &config).await.is_ok());
    }
}
