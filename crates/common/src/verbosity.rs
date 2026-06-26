//! Verbosity level for output filtering.
//!
//! Controls how much content is included in outbound messages.
//! Stored per-session and persisted in [`crate::session::persistence::SessionCheckpoint`].

use serde::{Deserialize, Serialize};

/// Verbosity level — controls outbound content filtering.
///
/// - `Full`: no filtering (default)
/// - `Normal`: remove `Thinking` content blocks
/// - `Off`: only keep `Text` content blocks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerbosityLevel {
    /// No filtering — include all content blocks.
    #[default]
    Full,
    /// Remove Thinking content blocks.
    Normal,
    /// Only keep Text content blocks.
    Off,
}

impl std::fmt::Display for VerbosityLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerbosityLevel::Full => write!(f, "full"),
            VerbosityLevel::Normal => write!(f, "normal"),
            VerbosityLevel::Off => write!(f, "off"),
        }
    }
}

impl std::str::FromStr for VerbosityLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "full" => Ok(VerbosityLevel::Full),
            "normal" => Ok(VerbosityLevel::Normal),
            "off" => Ok(VerbosityLevel::Off),
            _ => Err(format!("invalid verbosity level: {s}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_full() {
        assert_eq!(VerbosityLevel::default(), VerbosityLevel::Full);
    }

    #[test]
    fn test_display() {
        assert_eq!(VerbosityLevel::Full.to_string(), "full");
        assert_eq!(VerbosityLevel::Normal.to_string(), "normal");
        assert_eq!(VerbosityLevel::Off.to_string(), "off");
    }

    #[test]
    fn test_from_str() {
        assert_eq!(
            "full".parse::<VerbosityLevel>().unwrap(),
            VerbosityLevel::Full
        );
        assert_eq!(
            "normal".parse::<VerbosityLevel>().unwrap(),
            VerbosityLevel::Normal
        );
        assert_eq!(
            "off".parse::<VerbosityLevel>().unwrap(),
            VerbosityLevel::Off
        );
        assert!("invalid".parse::<VerbosityLevel>().is_err());
    }

    #[test]
    fn test_serde_roundtrip() {
        for level in [
            VerbosityLevel::Full,
            VerbosityLevel::Normal,
            VerbosityLevel::Off,
        ] {
            let json = serde_json::to_string(&level).unwrap();
            let parsed: VerbosityLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(level, parsed);
        }
    }

    #[test]
    fn test_serde_deserialize_from_string() {
        assert_eq!(
            serde_json::from_str::<VerbosityLevel>("\"full\"").unwrap(),
            VerbosityLevel::Full
        );
        assert_eq!(
            serde_json::from_str::<VerbosityLevel>("\"normal\"").unwrap(),
            VerbosityLevel::Normal
        );
        assert_eq!(
            serde_json::from_str::<VerbosityLevel>("\"off\"").unwrap(),
            VerbosityLevel::Off
        );
    }

    #[test]
    fn test_serde_invalid_value_fails() {
        assert!(serde_json::from_str::<VerbosityLevel>("\"extreme\"").is_err());
    }
}
