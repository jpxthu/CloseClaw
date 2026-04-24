//! Feishu Platform Adapter
//!
//! Core adapter implementing Stream→Plan fallback for Feishu platform.

use std::sync::Arc;

use crate::platform::capabilities::{PlatformCapabilityService, ReasoningMode};

use super::card::PlanCardConfig;
use super::card_updater::CardService;
#[cfg(test)]
use super::card_updater::{CardHandle, SectionUpdate};
use super::error::FeishuAdapterError;

/// Feishu message service trait (to be implemented by actual Feishu integration)
#[async_trait::async_trait]
pub trait FeishuMessageService: Send + Sync {
    /// Send a text message
    async fn send_message(&self, content: &str) -> Result<String, FeishuAdapterError>;

    /// Update an existing message
    async fn update_message(
        &self,
        message_id: &str,
        content: &str,
    ) -> Result<(), FeishuAdapterError>;

    /// Send a card message
    async fn send_card(&self, card_config: &PlanCardConfig) -> Result<String, FeishuAdapterError>;
}

/// Fallback result
#[derive(Debug, Clone)]
pub struct FallbackResult {
    /// Initial message ID
    pub initial_message_id: String,
    /// Card message ID
    pub card_message_id: String,
    /// Final message ID
    pub final_message_id: String,
}

/// Feishu adapter for handling platform-specific behavior
#[derive(Clone)]
pub struct FeishuAdapter {
    /// Platform capability service
    capability_service: Arc<PlatformCapabilityService>,
    /// Card service
    pub(crate) card_service: Arc<dyn CardService>,
    /// Message service
    pub(crate) message_service: Arc<dyn FeishuMessageService>,
    /// Whether fallback is enabled
    fallback_enabled: bool,
}

impl FeishuAdapter {
    /// Create a new FeishuAdapter
    pub fn new(
        capability_service: Arc<PlatformCapabilityService>,
        card_service: Arc<dyn CardService>,
        message_service: Arc<dyn FeishuMessageService>,
    ) -> Self {
        Self {
            capability_service,
            card_service,
            message_service,
            fallback_enabled: true,
        }
    }

    /// Check if fallback is needed for the given mode
    pub fn should_fallback(&self, mode: ReasoningMode) -> bool {
        mode == ReasoningMode::Stream
            && !self
                .capability_service
                .supports_mode_fully("feishu", ReasoningMode::Stream)
    }

    pub fn get_fallback_mode(&self) -> ReasoningMode {
        ReasoningMode::Plan
    }

    pub fn is_fallback_enabled(&self) -> bool {
        self.fallback_enabled
    }

    pub(crate) fn card_service(&self) -> &Arc<dyn CardService> {
        &self.card_service
    }

    pub(crate) fn message_service(&self) -> &Arc<dyn FeishuMessageService> {
        &self.message_service
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::feishu::card_updater::SectionUpdate;
    use crate::platform::PlatformCapabilityService;
    use async_trait::async_trait;

    struct MockCardService;
    struct MockMessageService;

    #[async_trait]
    impl CardService for MockCardService {
        async fn create_card(
            &self,
            _config: &PlanCardConfig,
        ) -> Result<CardHandle, FeishuAdapterError> {
            Ok(CardHandle {
                message_id: "mock_card_id".to_string(),
            })
        }

        async fn update_section(
            &self,
            _card_id: &str,
            _section_index: usize,
            _update: SectionUpdate,
        ) -> Result<(), FeishuAdapterError> {
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

    #[async_trait]
    impl FeishuMessageService for MockMessageService {
        async fn send_message(&self, _content: &str) -> Result<String, FeishuAdapterError> {
            Ok("mock_message_id".to_string())
        }

        async fn update_message(
            &self,
            _message_id: &str,
            _content: &str,
        ) -> Result<(), FeishuAdapterError> {
            Ok(())
        }

        async fn send_card(
            &self,
            _card_config: &PlanCardConfig,
        ) -> Result<String, FeishuAdapterError> {
            Ok("mock_card_id".to_string())
        }
    }

    fn create_test_adapter() -> FeishuAdapter {
        FeishuAdapter::new(
            Arc::new(PlatformCapabilityService::new()),
            Arc::new(MockCardService),
            Arc::new(MockMessageService),
        )
    }

    #[tokio::test]
    async fn test_should_fallback_for_stream() {
        let adapter = create_test_adapter();
        // Feishu stream support is Partial, so it should fallback
        assert!(adapter.should_fallback(ReasoningMode::Stream));
    }

    #[tokio::test]
    async fn test_should_not_fallback_for_direct() {
        let adapter = create_test_adapter();
        assert!(!adapter.should_fallback(ReasoningMode::Direct));
    }

    #[tokio::test]
    async fn test_get_fallback_mode() {
        let adapter = create_test_adapter();
        assert_eq!(adapter.get_fallback_mode(), ReasoningMode::Plan);
    }
}

/// Test-only constructor that allows controlling fallback_enabled.
/// This lives outside the main impl block so it can be used by sibling test modules
/// (e.g., adapter_event::tests) via pub(crate) visibility.
#[cfg(test)]
impl FeishuAdapter {
    pub(crate) fn new_for_test(
        capability_service: Arc<PlatformCapabilityService>,
        card_service: Arc<dyn CardService>,
        message_service: Arc<dyn FeishuMessageService>,
        fallback_enabled: bool,
    ) -> Self {
        Self {
            capability_service,
            card_service,
            message_service,
            fallback_enabled,
        }
    }
}
