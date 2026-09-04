//! Session Mode — controls session-level behavior constraints.
//!
//! `SessionMode` is **orthogonal** to `ReasoningMode` / `ReasoningModeState`:
//!
//! | Concept | Scope | Examples |
//! |---------|-------|----------|
//! | `ReasoningMode` | LLM reasoning presentation | Direct, Plan, Stream, Hidden |
//! | `ReasoningModeState` | LLM reasoning step tracking | current_step, is_complete |
//! | `SessionMode` | Session behavior constraints | Normal, Plan, Auto |
//!
//! `SessionMode` governs tool visibility, permission boundaries, and
//! system prompt instructions. `ReasoningMode` governs how the LLM
//! presents its thinking. They are stored independently and switched
//! independently.

use serde::{Deserialize, Serialize};

/// Session Mode — session-level behavior constraint.
///
/// Controls which system prompt instructions are injected and (in
/// future steps) which tools/permissions are active.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionMode {
    /// Default mode — no extra behavior constraints.
    #[default]
    Normal,
    /// Plan Mode — workflow: Research → Design → Review.
    Plan,
    /// Auto Mode — autonomous execution with approval gates.
    Auto,
}

impl std::fmt::Display for SessionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionMode::Normal => write!(f, "normal"),
            SessionMode::Plan => write!(f, "plan"),
            SessionMode::Auto => write!(f, "auto"),
        }
    }
}

impl SessionMode {
    /// Parse a mode string, returning `None` for unknown values.
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "normal" => Some(SessionMode::Normal),
            "plan" => Some(SessionMode::Plan),
            "auto" => Some(SessionMode::Auto),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_mode_default_is_normal() {
        assert_eq!(SessionMode::default(), SessionMode::Normal);
    }

    #[test]
    fn test_session_mode_display() {
        assert_eq!(SessionMode::Normal.to_string(), "normal");
        assert_eq!(SessionMode::Plan.to_string(), "plan");
        assert_eq!(SessionMode::Auto.to_string(), "auto");
    }

    #[test]
    fn test_session_mode_serde_roundtrip() {
        for mode in [SessionMode::Normal, SessionMode::Plan, SessionMode::Auto] {
            let json = serde_json::to_string(&mode).unwrap();
            let parsed: SessionMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, parsed);
        }
    }

    #[test]
    fn test_session_mode_from_str_opt() {
        assert_eq!(
            SessionMode::from_str_opt("normal"),
            Some(SessionMode::Normal)
        );
        assert_eq!(SessionMode::from_str_opt("plan"), Some(SessionMode::Plan));
        assert_eq!(SessionMode::from_str_opt("auto"), Some(SessionMode::Auto));
        assert_eq!(
            SessionMode::from_str_opt("NORMAL"),
            Some(SessionMode::Normal)
        );
        assert_eq!(SessionMode::from_str_opt("unknown"), None);
        assert_eq!(SessionMode::from_str_opt(""), None);
    }

    // ── Step 1.3 edge cases: from_str_opt boundary values ─────────────

    #[test]
    fn test_session_mode_from_str_opt_auto_case_insensitive() {
        // /execute path depends on from_str_opt("auto") returning Some(Auto).
        // Ensure various casings all resolve correctly.
        assert_eq!(SessionMode::from_str_opt("auto"), Some(SessionMode::Auto));
        assert_eq!(SessionMode::from_str_opt("AUTO"), Some(SessionMode::Auto));
        assert_eq!(SessionMode::from_str_opt("Auto"), Some(SessionMode::Auto));
        assert_eq!(SessionMode::from_str_opt("aUtO"), Some(SessionMode::Auto));
    }

    #[test]
    fn test_session_mode_from_str_opt_empty_and_whitespace() {
        // Empty and whitespace should return None (not panic)
        assert_eq!(SessionMode::from_str_opt(""), None);
        assert_eq!(SessionMode::from_str_opt(" "), None);
        assert_eq!(SessionMode::from_str_opt("  auto  "), None);
        assert_eq!(SessionMode::from_str_opt("\t"), None);
    }

    #[test]
    fn test_session_mode_from_str_opt_partial_match() {
        // Partial strings should NOT match
        assert_eq!(SessionMode::from_str_opt("aut"), None);
        assert_eq!(SessionMode::from_str_opt("pl"), None);
        assert_eq!(SessionMode::from_str_opt("norm"), None);
        assert_eq!(SessionMode::from_str_opt("automatic"), None);
    }

    #[test]
    fn test_session_mode_invalid_deserialize() {
        let result = serde_json::from_str::<SessionMode>("\"unknown\"");
        assert!(result.is_err());
    }
}
