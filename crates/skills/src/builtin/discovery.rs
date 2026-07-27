//! Skill discovery skill - allows agents to search and install skills from ClawHub
use crate::registry::{Skill, SkillManifest};
use async_trait::async_trait;

#[derive(Default)]
pub struct SkillDiscoverySkill;

impl SkillDiscoverySkill {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Skill for SkillDiscoverySkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "skill_discovery".to_string(),
            version: "1.0.0".to_string(),
            description: "Search, install, and manage skills from ClawHub marketplace. Use find to search, install to add, list to see installed, update to upgrade.".to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec!["clawhub".to_string()],
        }
    }

    fn body(&self) -> &str {
        r#"# Skill Discovery Skill

Use the `exec` tool to run `clawhub` CLI commands for skill management:

- **Search**: `exec` with `clawhub search <query>`
- **Install**: `exec` with `clawhub install <skill-name>` (optionally `--version <version>`)
- **List installed**: `exec` with `clawhub list`
- **Update**: `exec` with `clawhub update [skill-name]` (or `--all` for all skills)

Always confirm before installing or updating skills."#
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest() {
        let skill = SkillDiscoverySkill::new();
        let m = skill.manifest();
        assert_eq!(m.name, "skill_discovery");
        assert_eq!(m.version, "1.0.0");
        assert!(m.dependencies.contains(&"clawhub".to_string()));
    }

    #[test]
    fn test_body_not_empty() {
        let skill = SkillDiscoverySkill::new();
        let body = skill.body();
        assert!(!body.is_empty());
        assert!(body.contains("Skill Discovery Skill"));
    }

    #[test]
    fn test_default() {
        let skill = SkillDiscoverySkill::default();
        assert_eq!(skill.manifest().name, "skill_discovery");
    }
}
