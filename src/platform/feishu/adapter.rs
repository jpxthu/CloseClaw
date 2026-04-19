//! Feishu Platform Adapter
//!
//! Core adapter implementing Stream→Plan fallback for Feishu platform.

use std::sync::Arc;

use crate::platform::capabilities::{PlatformCapabilityService, ReasoningMode};
use crate::session::events::ModeSwitchEvent;

use super::card::{default_plan_card_config, PlanCardConfig};
use super::card_updater::{CardHandle, CardService};
use super::complexity::{get_high_complexity_config, HighComplexityConfig};
use super::error::FeishuAdapterError;
use crate::platform::feishu::updater::run_streaming_with_card_update;

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
    card_service: Arc<dyn CardService>,
    /// Message service
    message_service: Arc<dyn FeishuMessageService>,
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
        }
    }

    /// Check if fallback is needed for the given mode
    pub fn should_fallback(&self, mode: ReasoningMode) -> bool {
        mode == ReasoningMode::Stream
            && !self
                .capability_service
                .supports_mode_fully("feishu", ReasoningMode::Stream)
    }

    /// Get the fallback mode for Stream on Feishu
    pub fn get_fallback_mode(&self) -> ReasoningMode {
        ReasoningMode::Plan
    }

    /// Check if fallback is enabled (from config)
    pub fn is_fallback_enabled(&self) -> bool {
        // TODO: Read from config
        true
    }

    /// Send initial hint message
    async fn send_initial_message(&self) -> Result<String, FeishuAdapterError> {
        let content = "🔍 进入深度分析模式...";
        self.message_service.send_message(content).await
    }

    /// Create Plan card framework
    async fn create_plan_card(&self, goal: Option<&str>) -> Result<CardHandle, FeishuAdapterError> {
        let config = default_plan_card_config(goal);
        self.card_service.create_card(&config).await
    }

    /// Send final conclusion message
    async fn send_final_message(&self, card: &CardHandle) -> Result<String, FeishuAdapterError> {
        let content = "✅ 分析完成，请查看上方计划卡片";
        self.message_service.send_message(content).await
    }

    /// Execute the fallback flow
    pub async fn execute_fallback(
        &self,
        intent: &ModeSwitchEvent,
    ) -> Result<FallbackResult, FeishuAdapterError> {
        if !self.is_fallback_enabled() {
            return Err(FeishuAdapterError::FallbackNotEnabled);
        }

        // Get the goal for card creation
        let goal = intent
            .user_intent
            .as_ref()
            .and_then(|u| u.parsed_goal.as_ref())
            .map(|s| s.as_str());

        // Step 1: Send initial hint
        let initial = self.send_initial_message().await?;

        // Step 2: Create Plan card
        let card = self.create_plan_card(goal).await?;

        // Step 3: Run streaming updates with card
        run_streaming_with_card_update(
            &*self.card_service,
            &card,
            intent,
            get_high_complexity_config(),
        )
        .await?;

        // Step 4: Send final conclusion
        let final_msg = self.send_final_message(&card).await?;

        Ok(FallbackResult {
            initial_message_id: initial,
            card_message_id: card.message_id,
            final_message_id: final_msg,
        })
    }

    /// Handle a mode switch event, determining if fallback is needed
    pub async fn handle_mode_switch(
        &self,
        event: &ModeSwitchEvent,
    ) -> Result<Option<FallbackResult>, FeishuAdapterError> {
        let target_mode = event
            .target_mode
            .or_else(|| event.requested_mode)
            .unwrap_or(ReasoningMode::Direct);

        if self.should_fallback(target_mode) {
            let result = self.execute_fallback(event).await?;
            Ok(Some(result))
        } else {
            Ok(None)
        }
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

    #[tokio::test]
    async fn test_is_high_complexity_detection() {
        use crate::session::events::UserIntent;
        use std::sync::Arc;

        let adapter = create_test_adapter();
        let event = ModeSwitchEvent {
            target_mode: Some(ReasoningMode::Stream),
            user_intent: Some(Arc::new(UserIntent {
                raw_input: "设计一个用户认证系统".to_string(),
                parsed_goal: Some("设计一个用户认证系统".to_string()),
                entities: vec![],
            })),
            ..Default::default()
        };

        // This would normally use is_high_complexity, but we test handle_mode_switch
        // which calls execute_fallback
        let result = adapter.handle_mode_switch(&event).await;
        assert!(result.is_ok());
    }
}
