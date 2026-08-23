//! Mode transition detection for system prompt injection.

use crate::persistence::SessionMode;
use closeclaw_common::system_prompt::ModeTransition;

/// Pending mode transition type alias.
pub(crate) type PendingTransition = std::sync::Arc<std::sync::Mutex<Option<ModeTransition>>>;

/// Detect mode transition from previous to new mode.
pub(crate) fn detect(prev: SessionMode, new: SessionMode) -> Option<ModeTransition> {
    if prev == new {
        return None;
    }
    match (prev, new) {
        (_, SessionMode::Plan) => Some(ModeTransition::PlanModeReentry),
        (SessionMode::Plan, _) => Some(ModeTransition::PlanModeExit),
        (SessionMode::Auto, _) => Some(ModeTransition::AutoModeExit),
        _ => None,
    }
}
