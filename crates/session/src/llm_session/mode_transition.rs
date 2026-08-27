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

// ── Deferred mode switching (§6 design) ────────────────────────────────

impl super::ConversationSession {
    /// Store a pending mode change triggered by a slash command.
    ///
    /// The mode is applied lazily on the next [`session_mode()`] call,
    /// producing the one-message delay required by the design doc.
    pub fn set_pending_session_mode(&self, mode: SessionMode) {
        *self
            .pending_session_mode
            .lock()
            .expect("pending_session_mode lock poisoned") = Some(mode);
    }

    /// Check for a pending mode and apply it if present.
    /// Called by [`session_mode()`] to implement deferred mode switching.
    pub(crate) fn apply_pending_session_mode_if_needed(&self) {
        let pending = self
            .pending_session_mode
            .lock()
            .expect("pending_session_mode lock poisoned")
            .take();
        if let Some(mode) = pending {
            let prev = {
                let mut lock = self
                    .session_mode
                    .lock()
                    .expect("session_mode lock poisoned");
                let p = *lock;
                *lock = mode;
                p
            };
            let has_been = self
                .has_been_in_plan
                .load(std::sync::atomic::Ordering::Relaxed);
            if mode == SessionMode::Plan {
                self.has_been_in_plan
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            if let Some(t) = detect(prev, mode, has_been, ModeChangeSource::Manual) {
                *self
                    .pending_mode_transition
                    .lock()
                    .expect("pending_mode_transition lock poisoned") = Some(t);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_same_mode_returns_none() {
        assert_eq!(
            detect(
                SessionMode::Normal,
                SessionMode::Normal,
                false,
                ModeChangeSource::Automatic,
            ),
            None
        );
        assert_eq!(
            detect(
                SessionMode::Plan,
                SessionMode::Plan,
                true,
                ModeChangeSource::Manual,
            ),
            None
        );
    }

    #[test]
    fn test_first_entry_plan_no_transition() {
        // §6: first entry into Plan Mode → no transition
        assert_eq!(
            detect(
                SessionMode::Normal,
                SessionMode::Plan,
                false, // has_been_in_plan = false → first entry
                ModeChangeSource::Automatic,
            ),
            None
        );
    }

    #[test]
    fn test_reentry_plan_has_transition() {
        // §6: re-entering Plan Mode → PlanModeReentry
        assert_eq!(
            detect(
                SessionMode::Normal,
                SessionMode::Plan,
                true, // has_been_in_plan = true → re-entry
                ModeChangeSource::Automatic,
            ),
            Some(ModeTransition::PlanModeReentry)
        );
        // Also from Auto → Plan re-entry
        assert_eq!(
            detect(
                SessionMode::Auto,
                SessionMode::Plan,
                true,
                ModeChangeSource::Manual,
            ),
            Some(ModeTransition::PlanModeReentry)
        );
    }

    #[test]
    fn test_plan_exit_has_transition() {
        // Plan → Normal always produces PlanModeExit
        assert_eq!(
            detect(
                SessionMode::Plan,
                SessionMode::Normal,
                true,
                ModeChangeSource::Manual,
            ),
            Some(ModeTransition::PlanModeExit)
        );
        // Plan → Auto also produces PlanModeExit
        assert_eq!(
            detect(
                SessionMode::Plan,
                SessionMode::Auto,
                true,
                ModeChangeSource::Automatic,
            ),
            Some(ModeTransition::PlanModeExit)
        );
    }

    #[test]
    fn test_auto_manual_exit_no_transition() {
        // §8: manual exit from Auto Mode → no transition
        assert_eq!(
            detect(
                SessionMode::Auto,
                SessionMode::Normal,
                false,
                ModeChangeSource::Manual,
            ),
            None
        );
        assert_eq!(
            detect(
                SessionMode::Auto,
                SessionMode::Plan,
                false,
                ModeChangeSource::Manual,
            ),
            None
        );
    }

    #[test]
    fn test_auto_automatic_exit_has_transition() {
        // §8: automatic exit from Auto Mode → AutoModeExit
        // Auto → Normal is the standard automatic exit case
        assert_eq!(
            detect(
                SessionMode::Auto,
                SessionMode::Normal,
                false,
                ModeChangeSource::Automatic,
            ),
            Some(ModeTransition::AutoModeExit)
        );
        // Auto → Plan with has_been_in_plan=true: Plan re-entry takes precedence
        assert_eq!(
            detect(
                SessionMode::Auto,
                SessionMode::Plan,
                true, // has_been_in_plan=true → re-entry rule applies first
                ModeChangeSource::Automatic,
            ),
            Some(ModeTransition::PlanModeReentry)
        );
    }

    #[test]
    fn test_normal_to_auto_no_transition() {
        // Normal → Auto has no defined transition
        assert_eq!(
            detect(
                SessionMode::Normal,
                SessionMode::Auto,
                false,
                ModeChangeSource::Manual,
            ),
            None
        );
    }

    #[test]
    fn test_first_entry_plan_from_auto_no_transition() {
        // Auto → Plan (first entry) → no transition
        assert_eq!(
            detect(
                SessionMode::Auto,
                SessionMode::Plan,
                false,
                ModeChangeSource::Manual,
            ),
            None
        );
    }
}
