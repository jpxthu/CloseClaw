//! Skill Creator Skill - Help agents create new skills
//!
//! This skill helps agents create SKILL.md files for CloseClaw.

use crate::disk::types::SkillEffort;
use crate::registry::{Skill, SkillListingMeta, SkillManifest};
use async_trait::async_trait;

pub struct SkillCreatorSkill;

impl Default for SkillCreatorSkill {
    fn default() -> Self {
        Self
    }
}

impl SkillCreatorSkill {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Skill for SkillCreatorSkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "skill_creator".to_string(),
            version: "1.0.0".to_string(),
            description: "Helps agents understand how to create new skills for CloseClaw"
                .to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec![],
        }
    }

    fn body(&self) -> &str {
        r#"# Skill Creator

Use the `write` tool to create a SKILL.md file. The file must follow this format:

## SKILL.md Template

```markdown
---
description: One-line description of what this skill does.
when-to-use: Decision hints for when to invoke this skill.
paths:
  - "src/**/*.rs"
user-invocable: true
effort: small
---

# Skill Name

## Overview
Description of the skill purpose and capabilities.

## Instructions
Step-by-step instructions for the Agent to follow when this skill is invoked.

Use `${SKILL_DIR}` to reference the skill directory path (disk-based skills only).
Use `${SESSION_ID}` to reference the current session ID (disk-based skills only).
```

## Frontmatter Fields

| Field | Required | Description |
|-------|----------|-------------|
| description | Yes | Brief skill description for Agent decision-making |
| when-to-use | No | Decision hints for invocation timing |
| paths | No | File glob patterns for conditional activation |
| user-invocable | No | Allow direct user invocation via slash commands |
| effort | No | Cost estimate: small, medium, large |

## Validation

A valid SKILL.md must:
1. Have a `---` frontmatter block at the top
2. Include a `description` field in frontmatter
3. Have Markdown content after the frontmatter
4. Use proper YAML syntax in frontmatter"#
    }

    fn listing_meta(&self) -> SkillListingMeta {
        SkillListingMeta {
            when_to_use: "Use when the agent needs to create or understand how to create new skills for CloseClaw".to_string(),
            user_invocable: true,
            paths: vec![],
            effort: SkillEffort::Small,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest() {
        let skill = SkillCreatorSkill::new();
        let m = skill.manifest();
        assert_eq!(m.name, "skill_creator");
        assert_eq!(m.version, "1.0.0");
        assert!(!m.description.is_empty());
    }

    #[test]
    fn test_body_not_empty() {
        let skill = SkillCreatorSkill::new();
        let body = skill.body();
        assert!(!body.is_empty());
        assert!(body.contains("Skill Creator"));
        assert!(body.contains("SKILL.md Template"));
    }

    #[test]
    fn test_body_contains_frontmatter_guide() {
        let skill = SkillCreatorSkill::new();
        let body = skill.body();
        assert!(body.contains("description"));
        assert!(body.contains("Frontmatter Fields"));
    }

    #[test]
    fn test_default() {
        let skill = SkillCreatorSkill::default();
        assert_eq!(skill.manifest().name, "skill_creator");
    }
}
