//! Coding Agent Skill - Delegate coding tasks to AI coding agents
//!
//! This skill wraps OpenCode or Claude Code to handle complex coding tasks.

use crate::registry::{Skill, SkillManifest};
use async_trait::async_trait;

pub struct CodingAgentSkill;

impl CodingAgentSkill {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CodingAgentSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Skill for CodingAgentSkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "coding_agent".to_string(),
            version: "1.0.0".to_string(),
            description:
                "Delegate complex coding tasks to AI coding agents (OpenCode, Claude Code)"
                    .to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec![],
        }
    }

    fn body(&self) -> &str {
        r#"# Coding Agent Skill

Use the `exec` tool to delegate coding tasks to an AI coding agent.

- **Delegate**: `exec` with `opencode run "<task>"` or `claude-code "<task>"`
- **Review code**: Read the file first with `read`, then ask for review.
- **Refactor**: Read the file, then delegate the refactoring task.
- **Generate tests**: Read the source file, then delegate test generation.

Always read relevant files before delegating to provide context."#
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let skill = CodingAgentSkill::new();
        let m = skill.manifest();
        assert_eq!(m.name, "coding_agent");
    }

    #[test]
    fn test_manifest_fields() {
        let skill = CodingAgentSkill::new();
        let m = skill.manifest();
        assert_eq!(m.name, "coding_agent");
        assert_eq!(m.version, "1.0.0");
        assert!(m.description.contains("AI coding agents"));
        assert_eq!(m.author, Some("CloseClaw Team".to_string()));
        assert!(m.dependencies.is_empty());
    }

    #[test]
    fn test_body_not_empty() {
        let skill = CodingAgentSkill::new();
        let body = skill.body();
        assert!(!body.is_empty());
        assert!(body.contains("Coding Agent Skill"));
    }
}
