//! Mode transition detection for system prompt injection.

use crate::persistence::SessionMode;
use closeclaw_common::system_prompt::ModeTransition;

/// Pending mode transition type alias.
pub(crate) type PendingTransition = std::sync::Arc<std::sync::Mutex<Option<ModeTransition>>>;

/// Source of a mode change, distinguishing manual user actions from
/// automatic transitions (e.g. auto-execution completing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeChangeSource {
    /// Manual user action (slash command, pause button, etc.).
    Manual,
    /// Automatic transition (e.g. auto execution finished).
    Automatic,
}

/// Detect mode transition from previous to new mode.
///
/// - `has_been_in_plan`: whether the session has ever entered Plan Mode
///   before. First entry into Plan produces no transition (§6).
/// - `source`: whether the mode change was triggered manually or
///   automatically. Manual exits from Auto Mode produce no transition (§8).
pub(crate) fn detect(
    prev: SessionMode,
    new: SessionMode,
    has_been_in_plan: bool,
    source: ModeChangeSource,
) -> Option<ModeTransition> {
    if prev == new {
        return None;
    }
    match (prev, new) {
        // §6: first entry into Plan Mode → no transition
        (_, SessionMode::Plan) if !has_been_in_plan => None,
        // §6: re-entering Plan Mode → PlanModeReentry
        (_, SessionMode::Plan) => Some(ModeTransition::PlanModeReentry),
        (SessionMode::Plan, _) => Some(ModeTransition::PlanModeExit),
        // §8: manual exit from Auto Mode → no transition
        (SessionMode::Auto, _) if source == ModeChangeSource::Manual => None,
        // §8: automatic exit from Auto Mode → AutoModeExit
        (SessionMode::Auto, _) => Some(ModeTransition::AutoModeExit),
        _ => None,
    }
}
