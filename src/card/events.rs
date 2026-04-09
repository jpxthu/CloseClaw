//! Card event definitions — events generated from user interactions with cards.

use super::elements::CardAction;

/// Card event — represents a user action on a card.
#[derive(Debug, Clone)]
pub enum CardEvent {
    /// User confirmed the plan
    PlanConfirmed {
        session_id: String,
        card_message_id: String,
    },
    /// User cancelled the plan
    PlanCancelled {
        session_id: String,
    },
    /// User requested plan regeneration
    PlanRegenerate {
        session_id: String,
    },
    /// User toggled a step's collapsed state
    StepToggled {
        session_id: String,
        step_index: u32,
        collapsed: bool,
    },
}

impl CardEvent {
    /// Convert a card action into a card event.
    ///
    /// Returns `None` for actions that don't map to events (e.g., ExpandStep/CollapseStep
    /// without a session context).
    pub fn from_action(
        action: &CardAction,
        session_id: String,
        card_message_id: Option<String>,
    ) -> Option<Self> {
        match action {
            CardAction::Confirm => Some(CardEvent::PlanConfirmed {
                session_id,
                card_message_id: card_message_id.unwrap_or_default(),
            }),

            CardAction::Cancel => Some(CardEvent::PlanCancelled { session_id }),

            CardAction::Custom { payload } if payload == "regenerate" => {
                Some(CardEvent::PlanRegenerate { session_id })
            }

            CardAction::ExpandStep { step_index } => Some(CardEvent::StepToggled {
                session_id,
                step_index: *step_index,
                collapsed: false,
            }),

            CardAction::CollapseStep { step_index } => Some(CardEvent::StepToggled {
                session_id,
                step_index: *step_index,
                collapsed: true,
            }),

            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::elements::CardAction;

    #[test]
    fn test_from_action_confirm() {
        let action = CardAction::Confirm;
        let event = CardEvent::from_action(&action, "sess_123".to_string(), Some("msg_456".to_string()));

        match event {
            Some(CardEvent::PlanConfirmed { session_id, card_message_id }) => {
                assert_eq!(session_id, "sess_123");
                assert_eq!(card_message_id, "msg_456");
            }
            _ => panic!("Expected PlanConfirmed"),
        }
    }

    #[test]
    fn test_from_action_confirm_without_message_id() {
        let action = CardAction::Confirm;
        let event = CardEvent::from_action(&action, "sess_123".to_string(), None);

        match event {
            Some(CardEvent::PlanConfirmed { card_message_id, .. }) => {
                assert_eq!(card_message_id, "");
            }
            _ => panic!("Expected PlanConfirmed"),
        }
    }

    #[test]
    fn test_from_action_cancel() {
        let action = CardAction::Cancel;
        let event = CardEvent::from_action(&action, "sess_789".to_string(), None);

        match event {
            Some(CardEvent::PlanCancelled { session_id }) => {
                assert_eq!(session_id, "sess_789");
            }
            _ => panic!("Expected PlanCancelled"),
        }
    }

    #[test]
    fn test_from_action_regenerate() {
        let action = CardAction::Custom {
            payload: "regenerate".to_string(),
        };
        let event = CardEvent::from_action(&action, "sess_abc".to_string(), None);

        match event {
            Some(CardEvent::PlanRegenerate { session_id }) => {
                assert_eq!(session_id, "sess_abc");
            }
            _ => panic!("Expected PlanRegenerate"),
        }
    }

    #[test]
    fn test_from_action_expand_step() {
        let action = CardAction::ExpandStep { step_index: 2 };
        let event = CardEvent::from_action(&action, "sess_xyz".to_string(), None);

        match event {
            Some(CardEvent::StepToggled { session_id, step_index, collapsed }) => {
                assert_eq!(session_id, "sess_xyz");
                assert_eq!(step_index, 2);
                assert!(!collapsed);
            }
            _ => panic!("Expected StepToggled"),
        }
    }

    #[test]
    fn test_from_action_collapse_step() {
        let action = CardAction::CollapseStep { step_index: 5 };
        let event = CardEvent::from_action(&action, "sess_123".to_string(), None);

        match event {
            Some(CardEvent::StepToggled { step_index, collapsed, .. }) => {
                assert_eq!(step_index, 5);
                assert!(collapsed);
            }
            _ => panic!("Expected StepToggled"),
        }
    }

    #[test]
    fn test_from_action_unknown_custom() {
        // Custom action with unknown payload returns None
        let action = CardAction::Custom {
            payload: "unknown_action".to_string(),
        };
        let event = CardEvent::from_action(&action, "sess_123".to_string(), None);

        assert!(event.is_none());
    }
}
