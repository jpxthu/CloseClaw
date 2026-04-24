//! Fallback event handling for Feishu Adapter

use crate::platform::feishu::card::default_plan_card_config;
use crate::platform::feishu::card_updater::CardHandle;
use crate::platform::feishu::complexity::get_high_complexity_config;
use crate::platform::feishu::error::FeishuAdapterError;
use crate::platform::feishu::updater::run_streaming_with_card_update;
use crate::session::events::ModeSwitchEvent;

use super::adapter::FeishuAdapter;

impl FeishuAdapter {
    /// Send initial hint message
    async fn send_initial_message(&self) -> Result<String, FeishuAdapterError> {
        let content = "🔍 进入深度分析模式...";
        self.message_service().send_message(content).await
    }

    /// Create Plan card framework
    async fn create_plan_card(&self, goal: Option<&str>) -> Result<CardHandle, FeishuAdapterError> {
        let config = default_plan_card_config(goal);
        self.card_service().create_card(&config).await
    }

    /// Send final conclusion message
    async fn send_final_message(&self, _card: &CardHandle) -> Result<String, FeishuAdapterError> {
        let content = "✅ 分析完成，请查看上方计划卡片";
        self.message_service().send_message(content).await
    }

    /// Execute the fallback flow
    pub async fn execute_fallback(
        &self,
        intent: &ModeSwitchEvent,
    ) -> Result<crate::platform::feishu::adapter::FallbackResult, FeishuAdapterError> {
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
            &**self.card_service(),
            &card,
            intent,
            get_high_complexity_config(),
        )
        .await?;

        // Step 4: Send final conclusion
        let final_msg = self.send_final_message(&card).await?;

        Ok(crate::platform::feishu::adapter::FallbackResult {
            initial_message_id: initial,
            card_message_id: card.message_id,
            final_message_id: final_msg,
        })
    }

    /// Handle a mode switch event, determining if fallback is needed
    pub async fn handle_mode_switch(
        &self,
        event: &ModeSwitchEvent,
    ) -> Result<Option<crate::platform::feishu::adapter::FallbackResult>, FeishuAdapterError> {
        use crate::platform::capabilities::ReasoningMode;

        let target_mode = event
            .target_mode
            .or(event.requested_mode)
            .unwrap_or(ReasoningMode::Direct);

        if self.should_fallback(target_mode) {
            let result = self.execute_fallback(event).await?;
            Ok(Some(result))
        } else {
            Ok(None)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::capabilities::ReasoningMode;
    use crate::platform::feishu::card::PlanCardConfig;
    use crate::platform::feishu::card_updater::{CardService, SectionUpdate};
    use crate::platform::feishu::error::FeishuAdapterError;
    use crate::platform::PlatformCapabilityService;
    use async_trait::async_trait;
    use std::sync::Arc;

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
    impl crate::platform::feishu::adapter::FeishuMessageService for MockMessageService {
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

    /// Create a standard test adapter with fallback enabled
    fn create_test_adapter() -> FeishuAdapter {
        FeishuAdapter::new(
            Arc::new(PlatformCapabilityService::new()),
            Arc::new(MockCardService),
            Arc::new(MockMessageService),
        )
    }

    /// Create a test adapter with fallback disabled
    fn create_test_adapter_fallback_disabled() -> FeishuAdapter {
        FeishuAdapter::new_for_test(
            Arc::new(PlatformCapabilityService::new()),
            Arc::new(MockCardService),
            Arc::new(MockMessageService),
            false,
        )
    }

    // ---------------------------------------------------------------------------
    // execute_fallback tests
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn test_execute_fallback_success() {
        use crate::session::events::UserIntent;

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

        let result = adapter.execute_fallback(&event).await;
        assert!(result.is_ok(), "execute_fallback should succeed");

        let fallback = result.unwrap();
        // Verify all 4 steps produced message IDs
        assert_eq!(fallback.initial_message_id, "mock_message_id");
        assert_eq!(fallback.card_message_id, "mock_card_id");
        assert_eq!(fallback.final_message_id, "mock_message_id");
    }

    #[tokio::test]
    async fn test_execute_fallback_not_enabled() {
        use crate::session::events::UserIntent;

        let adapter = create_test_adapter_fallback_disabled();
        let event = ModeSwitchEvent {
            target_mode: Some(ReasoningMode::Stream),
            user_intent: Some(Arc::new(UserIntent {
                raw_input: "设计一个用户认证系统".to_string(),
                parsed_goal: Some("设计一个用户认证系统".to_string()),
                entities: vec![],
            })),
            ..Default::default()
        };

        let result = adapter.execute_fallback(&event).await;
        assert!(
            result.is_err(),
            "execute_fallback should fail when disabled"
        );
        assert!(matches!(
            result.unwrap_err(),
            FeishuAdapterError::FallbackNotEnabled
        ));
    }

    // ---------------------------------------------------------------------------
    // handle_mode_switch tests
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn test_handle_mode_switch_no_fallback_needed() {
        use crate::session::events::UserIntent;

        let adapter = create_test_adapter();
        // Direct mode does NOT need fallback
        let event = ModeSwitchEvent {
            target_mode: Some(ReasoningMode::Direct),
            user_intent: Some(Arc::new(UserIntent {
                raw_input: "简单问题".to_string(),
                parsed_goal: Some("简单问题".to_string()),
                entities: vec![],
            })),
            ..Default::default()
        };

        let result = adapter.handle_mode_switch(&event).await;
        assert!(result.is_ok());
        assert!(
            result.unwrap().is_none(),
            "Direct mode should not need fallback"
        );
    }

    #[tokio::test]
    async fn test_handle_mode_switch_fallback_needed() {
        use crate::session::events::UserIntent;

        let adapter = create_test_adapter();
        // Stream mode needs fallback (Feishu only partially supports Stream)
        let event = ModeSwitchEvent {
            target_mode: Some(ReasoningMode::Stream),
            user_intent: Some(Arc::new(UserIntent {
                raw_input: "设计一个用户认证系统".to_string(),
                parsed_goal: Some("设计一个用户认证系统".to_string()),
                entities: vec![],
            })),
            ..Default::default()
        };

        let result = adapter.handle_mode_switch(&event).await;
        assert!(result.is_ok());
        let fallback = result.unwrap();
        assert!(
            fallback.is_some(),
            "Stream mode should need fallback and return Some(FallbackResult)"
        );
        let fb = fallback.unwrap();
        assert_eq!(fb.initial_message_id, "mock_message_id");
        assert_eq!(fb.card_message_id, "mock_card_id");
        assert_eq!(fb.final_message_id, "mock_message_id");
    }
}
