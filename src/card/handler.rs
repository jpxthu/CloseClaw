//! Card action handler — processes card events and publishes them to the event bus.
//!
//! This module provides the integration between card button clicks and the
//! application's event system.

use super::events::CardEvent;

/// Card error types
#[derive(Debug, thiserror::Error)]
pub enum CardError {
    #[error("event bus error: {0}")]
    EventBus(String),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("invalid action: {0}")]
    InvalidAction(String),
}

/// Payload for plan confirmed event
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlanConfirmedPayload {
    pub session_id: String,
    pub card_message_id: String,
}

/// Payload for plan cancelled event
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlanCancelledPayload {
    pub session_id: String,
}

/// Payload for plan regeneration request
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlanRegeneratePayload {
    pub session_id: String,
}

/// Payload for step toggle event
#[derive(Debug, Clone, serde::Serialize)]
pub struct StepToggledPayload {
    pub session_id: String,
    pub step_index: u32,
    pub collapsed: bool,
}

/// Handle a card event and publish it to the event bus.
///
/// The actual event bus implementation is provided by the caller.
/// This function maps CardEvents to named events on the bus.
pub async fn handle_card_event(
    event: CardEvent,
    event_bus: &impl CardEventBus,
) -> Result<(), CardError> {
    match event {
        CardEvent::PlanConfirmed {
            session_id,
            card_message_id,
        } => {
            tracing::info!(
                session_id = %session_id,
                card_message_id = %card_message_id,
                "plan confirmed via card button"
            );
            event_bus
                .publish(
                    "plan_confirmed",
                    PlanConfirmedPayload {
                        session_id,
                        card_message_id,
                    },
                )
                .await
                .map_err(CardError::EventBus)
        }

        CardEvent::PlanCancelled { session_id } => {
            tracing::info!(
                session_id = %session_id,
                "plan cancelled via card button"
            );
            event_bus
                .publish(
                    "plan_cancelled",
                    PlanCancelledPayload { session_id },
                )
                .await
                .map_err(CardError::EventBus)
        }

        CardEvent::PlanRegenerate { session_id } => {
            tracing::info!(
                session_id = %session_id,
                "plan regeneration requested via card button"
            );
            event_bus
                .publish(
                    "plan_regenerate",
                    PlanRegeneratePayload { session_id },
                )
                .await
                .map_err(CardError::EventBus)
        }

        CardEvent::StepToggled {
            session_id,
            step_index,
            collapsed,
        } => {
            tracing::debug!(
                session_id = %session_id,
                step_index = %step_index,
                collapsed = %collapsed,
                "step toggled via card button"
            );
            event_bus
                .publish(
                    "step_toggled",
                    StepToggledPayload {
                        session_id,
                        step_index,
                        collapsed,
                    },
                )
                .await
                .map_err(CardError::EventBus)
        }
    }
}

/// Trait for the event bus — allows swapping implementations for testing.
#[async_trait::async_trait]
pub trait CardEventBus: Send + Sync {
    /// Publish an event with the given name and payload.
    async fn publish(&self, event_name: &str, payload: impl serde::Serialize + Send) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// A simple in-memory event bus for testing
    struct MockEventBus {
        events: Mutex<Vec<(String, String)>>,
    }

    impl MockEventBus {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }

        fn get_events(&self) -> Vec<(String, String)> {
            self.events.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl CardEventBus for MockEventBus {
        async fn publish(
            &self,
            event_name: &str,
            payload: impl serde::Serialize + Send,
        ) -> Result<(), String> {
            let payload_str =
                serde_json::to_string(&payload).map_err(|e| format!("serialize error: {}", e))?;
            self.events
                .lock()
                .unwrap()
                .push((event_name.to_string(), payload_str));
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_handle_plan_confirmed() {
        let bus = MockEventBus::new();
        let event = CardEvent::PlanConfirmed {
            session_id: "sess_123".to_string(),
            card_message_id: "msg_456".to_string(),
        };

        handle_card_event(event, &bus).await.unwrap();

        let events = bus.get_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "plan_confirmed");
        assert!(events[0].1.contains("sess_123"));
        assert!(events[0].1.contains("msg_456"));
    }

    #[tokio::test]
    async fn test_handle_plan_cancelled() {
        let bus = MockEventBus::new();
        let event = CardEvent::PlanCancelled {
            session_id: "sess_789".to_string(),
        };

        handle_card_event(event, &bus).await.unwrap();

        let events = bus.get_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "plan_cancelled");
        assert!(events[0].1.contains("sess_789"));
    }

    #[tokio::test]
    async fn test_handle_plan_regenerate() {
        let bus = MockEventBus::new();
        let event = CardEvent::PlanRegenerate {
            session_id: "sess_abc".to_string(),
        };

        handle_card_event(event, &bus).await.unwrap();

        let events = bus.get_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "plan_regenerate");
        assert!(events[0].1.contains("sess_abc"));
    }

    #[tokio::test]
    async fn test_handle_step_toggled() {
        let bus = MockEventBus::new();
        let event = CardEvent::StepToggled {
            session_id: "sess_xyz".to_string(),
            step_index: 3,
            collapsed: true,
        };

        handle_card_event(event, &bus).await.unwrap();

        let events = bus.get_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "step_toggled");
        assert!(events[0].1.contains("sess_xyz"));
        assert!(events[0].1.contains("3"));
        assert!(events[0].1.contains("true"));
    }
}
