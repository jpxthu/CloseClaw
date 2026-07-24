//! Disk Skill Types - type definitions for the disk skill system

use serde::{Deserialize, Serialize};
use std::fmt;

/// Source of a skill in the discovery hierarchy.
///
/// Lower variant index = higher priority (project overrides everything).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SkillSource {
    /// Project-local skills (highest priority).
    Project = 0,
    /// Agent-specific skills.
    Agent = 1,
    /// Global skills shared across all agents.
    Global = 2,
    /// Skills from user-provided extra directories.
    ExtraDirs = 3,
    /// Built-in skills bundled with the framework (lowest priority).
    Bundled = 4,
}

impl fmt::Display for SkillSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkillSource::Bundled => write!(f, "bundled"),
            SkillSource::ExtraDirs => write!(f, "extra_dirs"),
            SkillSource::Global => write!(f, "global"),
            SkillSource::Agent => write!(f, "agent"),
            SkillSource::Project => write!(f, "project"),
        }
    }
}

/// Context in which a skill is meant to run.
///
/// Skills always execute inline within the current Agent context.
/// No isolation or sub-agent creation occurs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillContext {
    /// Skill runs inline, without a dedicated agent context.
    #[default]
    Inline,
}

/// Effort level required to execute a skill.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillEffort {
    Trivial,
    Small,
    Medium,
    Large,
    #[default]
    Unknown,
}

impl fmt::Display for SkillEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkillEffort::Trivial => write!(f, "trivial"),
            SkillEffort::Small => write!(f, "small"),
            SkillEffort::Medium => write!(f, "medium"),
            SkillEffort::Large => write!(f, "large"),
            SkillEffort::Unknown => write!(f, "unknown"),
        }
    }
}

/// Manifest parsed from a SKILL.md frontmatter block.
///
/// Differs from [`crate::registry::SkillManifest`] which is the runtime
/// registry entry; this one is persisted in skill definition files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillManifest {
    /// Skill name. Used as the directory name on disk.
    #[serde(default)]
    pub name: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// When to use this skill.
    #[serde(default)]
    pub when_to_use: String,
    /// Execution context.
    #[serde(default)]
    pub context: SkillContext,
    /// Estimated effort.
    #[serde(default)]
    pub effort: SkillEffort,
    /// Additional search paths for subskills.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Whether the skill can be invoked directly by a user.
    #[serde(default)]
    pub user_invocable: bool,
}

/// A skill discovered on disk with its source and location.
#[derive(Debug, Clone)]
pub struct DiskSkill {
    /// Where this skill was found.
    pub source: SkillSource,
    /// Parsed manifest.
    pub manifest: SkillManifest,
    /// Absolute path to the SKILL.md file.
    pub readme_path: std::path::PathBuf,
    /// Absolute path to the skill directory.
    pub skill_dir: std::path::PathBuf,
}

impl DiskSkill {
    /// Load the skill body (instruction text) from the SKILL.md file on disk.
    ///
    /// Reads the file at `self.readme_path` and parses it with [`parse_skill_md`]
    /// to extract the body text (content after the frontmatter block).
    ///
    /// # Errors
    ///
    /// Returns an `io::Error` if the file cannot be read.
    pub fn load_body(&self) -> std::io::Result<String> {
        let raw = std::fs::read_to_string(&self.readme_path)?;
        let parsed = super::parse_skill_md(&raw).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to parse SKILL.md: {}", e),
            )
        })?;
        Ok(parsed.body)
    }
}

/// Result of parsing a SKILL.md frontmatter block.
#[derive(Debug, Clone)]
pub struct ParsedSkill {
    /// Parsed manifest fields.
    pub manifest: SkillManifest,
    /// If true, only the `description` field was present.
    pub description_only: bool,
    /// Raw frontmatter block (without delimiters), kept for traceability.
    pub frontmatter_raw: String,
    /// Skill body (instruction text) extracted from after the frontmatter.
    pub body: String,
}

/// Errors that can occur when parsing SKILL.md frontmatter.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    /// Frontmatter `---` opening delimiter is missing.
    #[error("frontmatter opening delimiter '---' not found")]
    MissingDelimiter,

    /// Frontmatter YAML could not be parsed.
    #[error("invalid frontmatter YAML: {0}")]
    InvalidYaml(String),

    /// Required `description` field is missing.
    #[error("required field 'description' is missing")]
    MissingDescription,
}

/// Configuration for the skill directory scanner.
#[derive(Debug, Clone, Default)]
pub struct ScanConfig {
    /// Directory containing bundled skills.
    pub bundled_dir: Option<std::path::PathBuf>,
    /// Additional directories to scan.
    pub extra_dirs: Vec<std::path::PathBuf>,
    /// Global skills directory.
    pub global_dir: Option<std::path::PathBuf>,
    /// Project-root skills directory.
    pub project_root: Option<std::path::PathBuf>,
    /// Agent id used to derive the agent-specific skills directory.
    pub agent_id: Option<String>,
    /// Explicit path to the agent-specific skills directory.
    /// When set, takes precedence over deriving from `agent_id` + `global_dir`.
    pub agent_skills_dir: Option<std::path::PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_source_priority() {
        assert!(SkillSource::Project < SkillSource::Agent);
        assert!(SkillSource::Agent < SkillSource::Global);
        assert!(SkillSource::Global < SkillSource::ExtraDirs);
        assert!(SkillSource::ExtraDirs < SkillSource::Bundled);
    }

    #[test]
    fn test_skill_source_display() {
        assert_eq!(SkillSource::Bundled.to_string(), "bundled");
        assert_eq!(SkillSource::ExtraDirs.to_string(), "extra_dirs");
        assert_eq!(SkillSource::Global.to_string(), "global");
        assert_eq!(SkillSource::Agent.to_string(), "agent");
        assert_eq!(SkillSource::Project.to_string(), "project");
    }

    #[test]
    fn test_skill_context_default() {
        let ctx = SkillContext::default();
        assert_eq!(ctx, SkillContext::Inline);
    }

    #[test]
    fn test_skill_effort_default() {
        assert_eq!(SkillEffort::default(), SkillEffort::Unknown);
    }

    #[test]
    fn test_skill_manifest_default() {
        let m = SkillManifest {
            name: "test".to_string(),
            description: "a test skill".to_string(),
            when_to_use: String::new(),
            context: SkillContext::Inline,
            effort: SkillEffort::default(),
            paths: vec![],
            user_invocable: false,
        };
        assert_eq!(m.name, "test");
        assert_eq!(m.context, SkillContext::Inline);
        assert!(!m.user_invocable);
    }

    #[test]
    fn test_parse_error_display() {
        assert_eq!(
            ParseError::MissingDelimiter.to_string(),
            "frontmatter opening delimiter '---' not found"
        );
        assert!(ParseError::InvalidYaml("oops".to_string())
            .to_string()
            .contains("invalid frontmatter YAML"));
        assert_eq!(
            ParseError::MissingDescription.to_string(),
            "required field 'description' is missing"
        );
    }

    #[test]
    fn test_scan_config_default() {
        let cfg = ScanConfig::default();
        assert!(cfg.bundled_dir.is_none());
        assert!(cfg.extra_dirs.is_empty());
        assert!(cfg.global_dir.is_none());
        assert!(cfg.project_root.is_none());
        assert!(cfg.agent_id.is_none());
        assert!(cfg.agent_skills_dir.is_none());
    }
}
