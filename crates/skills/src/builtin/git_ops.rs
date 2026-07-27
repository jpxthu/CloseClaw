//! Git operations skill
use crate::registry::{Skill, SkillManifest};
use async_trait::async_trait;

#[derive(Default)]
pub struct GitOpsSkill;

impl GitOpsSkill {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Skill for GitOpsSkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "git_ops".to_string(),
            version: "1.0.0".to_string(),
            description: "Git operations: status, commit, push, pull".to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec![],
        }
    }

    fn body(&self) -> &str {
        r#"# Git Operations Skill

You have access to the `exec` tool. Use it to run git commands:

- **Status**: `exec` with `git status --porcelain`
- **Log**: `exec` with `git log --oneline -10`
- **Commit**: `exec` with `git commit -m "<message>"`
- **Push**: `exec` with `git push`
- **Pull**: `exec` with `git pull`
- **Diff**: `exec` with `git diff`

Always ensure changes are staged before committing. Confirm destructive operations (force push, reset) with the user."#
    }
}
