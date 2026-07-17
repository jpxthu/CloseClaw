//! Mode transition types for system prompt injection.
//!
//! `ModeTransition` represents a one-shot notification injected into
//! the system prompt when the session transitions between modes.
//! It is defined in `closeclaw-common` (Layer 0) so both `session`
//! and `system_prompt` crates can reference it without violating
//! the layering rules.
//!
//! See `docs/design/mode/references/prompts.md` section 6 for the
//! transition prompt content.

/// Mode transition type — design doc section 6.
///
/// Each variant corresponds to a specific mode transition event that
/// requires a one-shot system prompt injection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeTransition {
    /// Re-entering Plan Mode after having previously exited it.
    Reentry,
    /// Exited Plan Mode (approval passed).
    ExitPlan,
    /// Exited Auto Mode.
    ExitAuto,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_transition_debug() {
        assert_eq!(format!("{:?}", ModeTransition::Reentry), "Reentry");
        assert_eq!(format!("{:?}", ModeTransition::ExitPlan), "ExitPlan");
        assert_eq!(format!("{:?}", ModeTransition::ExitAuto), "ExitAuto");
    }

    #[test]
    fn test_mode_transition_clone() {
        let t = ModeTransition::ExitPlan;
        let t2 = t;
        assert_eq!(t, t2);
    }

    #[test]
    fn test_mode_transition_partial_eq() {
        assert_eq!(ModeTransition::Reentry, ModeTransition::Reentry);
        assert_ne!(ModeTransition::Reentry, ModeTransition::ExitPlan);
        assert_ne!(ModeTransition::ExitPlan, ModeTransition::ExitAuto);
    }
}
