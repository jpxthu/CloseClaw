//! Disk Skill Types - type definitions for the disk skill system

use super::frontmatter::extract_skill_body;
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillContext {
    /// Skill runs inline, without a dedicated agent context.
    #[default]
    Inline,
    /// Skill runs within a specific agent.
    Agent {
        /// Agent identifier.
        agent_id: String,
    },
    /// Skill signals a fork: the caller should execute it in an
    /// isolated sub-agent rather than inline.
    Fork,
}

/// Effort level required to execute a skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillEffort {
    Trivial,
    Small,
    Medium,
    Large,
    Unknown,
}

impl Default for SkillEffort {
    fn default() -> Self {
        SkillEffort::Unknown
    }
}

/// Manifest parsed from a SKILL.md frontmatter block.
///
/// Differs from [`crate::skills::SkillManifest`] which is the runtime
/// registry entry; this one is persisted in skill definition files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillManifest {
    /// Skill name. Used as the directory name on disk.
    #[serde(default)]
    pub name: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Tools the skill is allowed to use.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// When to use this skill.
    #[serde(default)]
    pub when_to_use: String,
    /// Execution context.
    #[serde(default)]
    pub context: SkillContext,
    /// Agent type required to run this skill.
    #[serde(default)]
    pub agent: String,
    /// Explicit agent id for agent-scoped skills.
    #[serde(default)]
    pub agent_id: String,
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

/// Result of parsing a SKILL.md frontmatter block.
#[derive(Debug, Clone)]
pub struct ParsedSkill {
    /// Parsed manifest fields.
    pub manifest: SkillManifest,
    /// If true, only the `description` field was present.
    pub description_only: bool,
    /// Raw frontmatter block (without delimiters), kept for traceability.
    pub frontmatter_raw: String,
}

/// Errors that can occur when loading the skill body from disk.
#[derive(Debug, thiserror::Error)]
pub enum LoadBodyError {
    /// Failed to read the SKILL.md file from disk.
    #[error("failed to read SKILL.md: {0}")]
    Io(String),
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
}

impl DiskSkill {
    /// Load the skill body (instruction text) from disk on demand.
    ///
    /// Reads the SKILL.md file at `self.readme_path`, extracts the
    /// text after the frontmatter closing `---` delimiter, and returns
    /// the trimmed result.
    pub fn load_body(&self) -> Result<String, LoadBodyError> {
        let raw = std::fs::read_to_string(&self.readme_path)
            .map_err(|e| LoadBodyError::Io(e.to_string()))?;
        let body = extract_skill_body(&raw);
        Ok(body.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_skill(path: std::path::PathBuf) -> DiskSkill {
        DiskSkill {
            source: SkillSource::Global,
            manifest: SkillManifest {
                name: "test".into(),
                description: "test".into(),
                ..Default::default()
            },
            readme_path: path,
            skill_dir: std::path::PathBuf::new(),
        }
    }

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
    fn test_skill_context_agent() {
        let ctx = SkillContext::Agent {
            agent_id: "my-agent".to_string(),
        };
        assert!(matches!(ctx, SkillContext::Agent { .. }));
    }

    #[test]
    fn test_skill_context_fork() {
        let ctx = SkillContext::Fork;
        assert!(matches!(ctx, SkillContext::Fork));
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
            allowed_tools: vec![],
            when_to_use: String::new(),
            context: SkillContext::Inline,
            agent: String::new(),
            agent_id: String::new(),
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
    }

    #[test]
    fn test_load_body_existing_file() {
        let dir = std::env::temp_dir().join(format!("load_body_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("SKILL.md");
        let content = "---\ndescription: Test\n---\n\n# Hello\n\nInstructions here.\n";
        std::fs::write(&path, content).unwrap();

        let skill = make_skill(path.clone());
        let body = skill.load_body().unwrap();
        assert_eq!(body, "# Hello\n\nInstructions here.");

        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn test_load_body_missing_file() {
        let path = std::path::PathBuf::from("/nonexistent/SKILL.md");
        let skill = make_skill(path);
        let err = skill.load_body().unwrap_err();
        assert!(matches!(err, LoadBodyError::Io(_)));
    }

    #[test]
    fn test_load_body_no_frontmatter() {
        let dir = std::env::temp_dir().join(format!("load_body_no_fm_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("SKILL.md");
        let content = "Just plain text with no frontmatter.";
        std::fs::write(&path, content).unwrap();

        let skill = make_skill(path.clone());
        let body = skill.load_body().unwrap();
        assert_eq!(body, "");

        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
