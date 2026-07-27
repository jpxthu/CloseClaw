//! File operations skill
use crate::registry::{Skill, SkillManifest};
use async_trait::async_trait;

pub struct FileOpsSkill;

impl Default for FileOpsSkill {
    fn default() -> Self {
        Self::new()
    }
}

impl FileOpsSkill {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Skill for FileOpsSkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "file_ops".to_string(),
            version: "1.0.0".to_string(),
            description: "File system operations: read, write, list, delete".to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec![],
        }
    }

    fn body(&self) -> &str {
        r#"# File Operations Skill

You have access to file system tools. Use them to perform file operations:

- **Read a file**: Use the `read` tool with the file path.
- **Write a file**: Use the `write` tool with the file path and content.
- **Check if a file exists**: Use the `read` tool and check for errors, or use `exec` with `test -f <path>`.
- **List directory contents**: Use `exec` with `ls <path>` or `find <path> -maxdepth 1`.
- **Delete a file**: Use `exec` with `rm <path>`.

Always confirm destructive operations with the user before executing."#
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest() {
        let skill = FileOpsSkill::new();
        let m = skill.manifest();
        assert_eq!(m.name, "file_ops");
        assert_eq!(m.version, "1.0.0");
        assert!(!m.description.is_empty());
    }

    #[test]
    fn test_body_not_empty() {
        let skill = FileOpsSkill::new();
        let body = skill.body();
        assert!(!body.is_empty());
        assert!(body.contains("File Operations Skill"));
    }

    #[test]
    fn test_default() {
        let skill = FileOpsSkill::default();
        let m = skill.manifest();
        assert_eq!(m.name, "file_ops");
    }
}
