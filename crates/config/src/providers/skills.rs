//! Global skills configuration provider.
//!
//! Loads and validates the top-level `skills.json` config file that
//! provides global skill settings (extra directories for external
//! skill reuse). Per the design doc, this is an optional section —
//! absent file uses defaults and does not block daemon startup.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::providers::ConfigError;
use crate::ConfigProvider;

/// Skill-related settings.
///
/// Mirrors the original `SkillsConfig` that was embedded in
/// `SystemConfigData`.  Serde attributes (camelCase + default)
/// are preserved for backward compatibility with existing
/// `system.json` files.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillsConfig {
    /// Extra directories to scan for skills (appended after Global layer).
    /// Paths that do not exist are silently skipped at scan time.
    #[serde(default)]
    pub extra_dirs: Vec<String>,
}

/// Wrapper around [`SkillsConfig`] that implements the [`ConfigProvider`] trait.
///
/// The global `skills.json` has a flat schema.  All fields default to
/// empty/absent when the file is absent or empty (matching
/// `SkillsConfig::default()`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SkillsConfigData {
    #[serde(flatten)]
    pub config: SkillsConfig,
}

impl SkillsConfigData {
    /// Parse from a JSON string (useful for testing).
    pub fn from_json_str(content: &str) -> Result<Self, ConfigError> {
        let data: SkillsConfigData = serde_json::from_str(content)?;
        Ok(data)
    }

    /// Load from a file path.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path)?;
        Self::from_json_str(&content)
    }
}

impl ConfigProvider for SkillsConfigData {
    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn validate(&self) -> Result<(), ConfigError> {
        // Structural validation: serde already ensures required fields
        // are present with correct types. Additional business rules
        // can be added here.
        Ok(())
    }

    fn config_path() -> &'static str
    where
        Self: Sized,
    {
        "skills.json"
    }

    fn is_default(&self) -> bool {
        self.config.extra_dirs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helper ──────────────────────────────────────────────────────────

    fn parse(json: &str) -> SkillsConfigData {
        SkillsConfigData::from_json_str(json).expect("should parse")
    }

    // ── Default value ───────────────────────────────────────────────────

    #[test]
    fn test_default_extra_dirs_empty() {
        let data = SkillsConfigData::default();
        assert!(data.config.extra_dirs.is_empty());
    }

    #[test]
    fn test_skills_config_default() {
        let cfg = SkillsConfig::default();
        assert!(cfg.extra_dirs.is_empty());
    }

    // ── JSON roundtrip (camelCase extraDirs) ────────────────────────────

    #[test]
    fn test_roundtrip_preserves_extra_dirs() {
        let json = r#"{"extraDirs": ["/opt/skills", "~/my-skills"]}"#;
        let data = parse(json);
        assert_eq!(data.config.extra_dirs, vec!["/opt/skills", "~/my-skills"]);
        let serialized = serde_json::to_string(&data).unwrap();
        let deserialized: SkillsConfigData = serde_json::from_str(&serialized).unwrap();
        assert_eq!(data, deserialized);
    }

    #[test]
    fn test_roundtrip_empty_extra_dirs() {
        let json = r#"{"extraDirs": []}"#;
        let data = parse(json);
        assert!(data.config.extra_dirs.is_empty());
        let serialized = serde_json::to_string(&data).unwrap();
        let deserialized: SkillsConfigData = serde_json::from_str(&serialized).unwrap();
        assert_eq!(data, deserialized);
    }

    #[test]
    fn test_empty_object_uses_defaults() {
        let data = parse("{}");
        assert!(data.config.extra_dirs.is_empty());
        assert!(data.is_default());
    }

    // ── from_json_str error paths ───────────────────────────────────────

    #[test]
    fn test_empty_string_not_valid_json() {
        let result = SkillsConfigData::from_json_str("");
        assert!(result.is_err());
    }

    #[test]
    fn test_array_not_valid_object() {
        let result = SkillsConfigData::from_json_str("[]");
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_json_syntax() {
        let result = SkillsConfigData::from_json_str("{invalid}");
        assert!(result.is_err());
    }

    // ── ConfigProvider trait compliance ─────────────────────────────────

    #[test]
    fn test_version_string() {
        let data = SkillsConfigData::default();
        assert_eq!(data.version(), "1.0.0");
    }

    #[test]
    fn test_validate_always_succeeds() {
        let data = SkillsConfigData::default();
        assert!(data.validate().is_ok());
    }

    #[test]
    fn test_config_path() {
        assert_eq!(SkillsConfigData::config_path(), "skills.json");
    }

    #[test]
    fn test_is_default_empty_extra_dirs() {
        let data = SkillsConfigData::default();
        assert!(data.is_default());
    }

    #[test]
    fn test_is_default_non_empty_extra_dirs() {
        let json = r#"{"extraDirs": ["/opt/skills"]}"#;
        let data = parse(json);
        assert!(!data.is_default());
    }

    // ── Validate with invalid types ─────────────────────────────────────

    #[test]
    fn test_validate_rejects_non_string_array() {
        let json = r#"{"extraDirs": [123, true]}"#;
        let result = SkillsConfigData::from_json_str(json);
        assert!(result.is_err(), "non-string array elements should fail");
    }

    #[test]
    fn test_validate_extra_dirs_with_tilde() {
        let json = r#"{"extraDirs": ["~/skills", "~/more-skills"]}"#;
        let data = parse(json);
        assert_eq!(data.config.extra_dirs.len(), 2);
        assert!(data.config.extra_dirs[0].starts_with("~/"));
    }

    // ── from_file paths ─────────────────────────────────────────────────

    #[test]
    fn test_from_file_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("skills.json");
        fs::write(&path, r#"{"extraDirs": ["/opt/skills"]}"#).unwrap();
        let data = SkillsConfigData::from_file(&path).unwrap();
        assert_eq!(data.config.extra_dirs, vec!["/opt/skills"]);
    }

    #[test]
    fn test_from_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("skills.json");
        let result = SkillsConfigData::from_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_from_file_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("skills.json");
        fs::write(&path, "not json").unwrap();
        let result = SkillsConfigData::from_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_from_json_str_valid() {
        let data = SkillsConfigData::from_json_str(r#"{"extraDirs": []}"#).unwrap();
        assert!(data.config.extra_dirs.is_empty());
    }

    #[test]
    fn test_from_json_str_invalid() {
        let result = SkillsConfigData::from_json_str("invalid");
        assert!(result.is_err());
    }
}
