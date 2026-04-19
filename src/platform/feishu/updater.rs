//! Streaming Update Logic for Feishu Cards
//!
//! Implements the card update flow that simulates streaming behavior on Feishu.

use crate::session::events::ModeSwitchEvent;

use super::card::{build_initial_sections, StepStatus};
use super::card_updater::{CardService, SectionUpdate};
use super::complexity::HighComplexityConfig;
use super::error::FeishuAdapterError;

/// Run the streaming simulation with card updates
///
/// This simulates the effect of LLM streaming output by updating card sections
/// as each step progresses.
pub async fn run_streaming_with_card_update<C: CardService + ?Sized>(
    card_service: &C,
    card: &super::card_updater::CardHandle,
    intent: &ModeSwitchEvent,
    config: HighComplexityConfig,
) -> Result<(), FeishuAdapterError> {
    let goal = intent
        .user_intent
        .as_ref()
        .and_then(|u| u.parsed_goal.as_ref())
        .map(|s| s.as_str());

    let sections = build_initial_sections(goal);
    let total_steps = sections.len() as u32;

    // Update progress to first step
    card_service
        .update_progress(&card.message_id, 1, total_steps)
        .await?;

    // Process each section
    for (idx, section) in sections.iter().enumerate() {
        // Mark current step as active
        let active_update = SectionUpdate {
            status: Some(StepStatus::Active),
            ..Default::default()
        };
        card_service
            .update_section(&card.message_id, idx, active_update)
            .await?;

        // In a real implementation, this is where LLM output would be streamed
        // and each complete sentence would trigger an update

        // Mark step as completed
        card_service
            .mark_step_complete(&card.message_id, section.step_number)
            .await?;
    }

    // Final progress update
    card_service
        .update_progress(&card.message_id, total_steps, total_steps)
        .await?;

    Ok(())
}

/// Update card content during streaming
///
/// Called by LLM service when a complete thought/segment is ready.
pub async fn update_card_content<C: CardService + ?Sized>(
    card_service: &C,
    card_id: &str,
    section_index: usize,
    content: &str,
) -> Result<(), FeishuAdapterError> {
    let update = SectionUpdate {
        content: Some(content.to_string()),
        ..Default::default()
    };
    card_service
        .update_section(card_id, section_index, update)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::capabilities::PlatformCapabilityService;
    use crate::session::events::UserIntent;
    use async_trait::async_trait;
    use std::sync::Arc;

    use super::super::card::{PlanCardConfig, PlanSection};
    use super::super::card_updater::{CardHandle, CardService, SectionUpdate};

    struct MockCardService {
        updates: std::sync::Mutex<Vec<String>>,
    }

    impl MockCardService {
        fn new() -> Self {
            Self {
                updates: std::sync::Mutex::new(vec![]),
            }
        }
    }

    #[async_trait]
    impl CardService for MockCardService {
        async fn create_card(
            &self,
            _config: &PlanCardConfig,
        ) -> Result<CardHandle, FeishuAdapterError> {
            Ok(CardHandle {
                message_id: "mock_card".to_string(),
            })
        }

        async fn update_section(
            &self,
            _card_id: &str,
            section_index: usize,
            update: SectionUpdate,
        ) -> Result<(), FeishuAdapterError> {
            let mut updates = self.updates.lock().unwrap();
            updates.push(format!("section_{}: {:?}", section_index, update));
            Ok(())
        }

        async fn update_progress(
            &self,
            _card_id: &str,
            current: u32,
            total: u32,
        ) -> Result<(), FeishuAdapterError> {
            let mut updates = self.updates.lock().unwrap();
            updates.push(format!("progress_{}/{}", current, total));
            Ok(())
        }

        async fn mark_step_complete(
            &self,
            _card_id: &str,
            step: u32,
        ) -> Result<(), FeishuAdapterError> {
            let mut updates = self.updates.lock().unwrap();
            updates.push(format!("complete_{}", step));
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
    async fn test_run_streaming_with_card_update() {
        let mock = Arc::new(MockCardService::new());
        let card = CardHandle {
            message_id: "test_card".to_string(),
        };

        let intent = ModeSwitchEvent {
            user_intent: Some(Arc::new(UserIntent {
                raw_input: "测试目标".to_string(),
                parsed_goal: Some("测试目标".to_string()),
                entities: vec![],
            })),
            ..Default::default()
        };

        let config = HighComplexityConfig::default();
        let result = run_streaming_with_card_update(&*mock, &card, &intent, config).await;
        assert!(result.is_ok());

        let updates = mock.updates.lock().unwrap();
        assert!(!updates.is_empty());
    }
}
