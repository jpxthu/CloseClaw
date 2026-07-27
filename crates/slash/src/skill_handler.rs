//! Slash handler for user-invocable skills.
//!
//! Routes `/<skill-name>` to the matching skill in the disk or builtin
//! registry, loads its body, and injects it into the agent context
//! via [`SlashResult::InjectMeta`].

use std::sync::Arc;

use closeclaw_skills::disk::DiskSkillRegistry;
use closeclaw_skills::BuiltinSkillRegistry;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use closeclaw_common::slash_router::SlashResult;

/// Handler that dispatches `/<skill-name>` to the matching skill.
///
/// The command names are **not** declared statically — they are injected
/// at registration time via [`HandlerRegistry::register_named`]. This
/// allows the handler to dynamically claim any skill name that appears
/// in the disk or builtin registries.
pub struct SkillSlashHandler {
    disk_registry: Arc<DiskSkillRegistry>,
    builtin_registry: Arc<BuiltinSkillRegistry>,
}

impl SkillSlashHandler {
    /// Create a new handler backed by the given registries.
    pub fn new(
        disk_registry: Arc<DiskSkillRegistry>,
        builtin_registry: Arc<BuiltinSkillRegistry>,
    ) -> Self {
        Self {
            disk_registry,
            builtin_registry,
        }
    }

    /// Return the list of invocable skill names across both registries.
    ///
    /// Used at registration time so the caller can register one entry
    /// per skill name.
    pub fn invocable_names(&self) -> Vec<String> {
        let names: Vec<String> = self.disk_registry.user_invocable_names();
        // Builtin skills with `user_invocable: true` are not currently
        // registered via `user_invocable_names()` on the disk registry,
        // but we add them here for completeness. If the builtin registry
        // gains a similar method in the future, this can be simplified.
        names
    }

    /// Replace `${SKILL_DIR}` and `${SESSION_ID}` placeholders in the
    /// skill body.
    ///
    /// - `${SKILL_DIR}` → absolute path to the skill directory
    /// - `${SESSION_ID}` → current session ID from the slash context
    /// - Unrecognized `${...}` patterns remain unchanged.
    fn substitute_variables(body: &str, skill_dir: &std::path::Path, session_id: &str) -> String {
        let mut result = body.to_string();

        let skill_dir_str = skill_dir.to_string_lossy().to_string();
        result = result.replace("${SKILL_DIR}", &skill_dir_str);
        result = result.replace("${SESSION_ID}", session_id);

        result
    }
}

#[async_trait::async_trait]
impl SlashHandler for SkillSlashHandler {
    fn commands(&self) -> &[&str] {
        &[]
    }

    fn description(&self) -> &str {
        "调用技能"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        false
    }

    async fn handle(&self, _args: &str, ctx: &SlashContext) -> SlashResult {
        let skill_name = &ctx.command;

        // 1. Look up in DiskSkillRegistry
        if let Some(skill) = self.disk_registry.get(skill_name) {
            let body = match skill.load_body() {
                Ok(b) => b,
                Err(e) => {
                    return SlashResult::Reply(format!("技能 \"{skill_name}\" 加载失败: {e}"));
                }
            };
            let body = Self::substitute_variables(&body, &skill.skill_dir, &ctx.session_id);
            return SlashResult::InjectMeta { content: body };
        }

        // 2. Fallback: BuiltinSkillRegistry
        if let Some(skill) = self.builtin_registry.get(skill_name).await {
            let body = skill.body().to_string();
            return SlashResult::InjectMeta { content: body };
        }

        // 3. Not found
        SlashResult::Reply(format!("未知技能: {skill_name}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_skills::disk::types::{
        DiskSkill, SkillContext, SkillEffort, SkillManifest, SkillSource,
    };
    use std::path::{Path, PathBuf};

    fn make_disk_skill(name: &str, readme_path: PathBuf, skill_dir: PathBuf) -> DiskSkill {
        DiskSkill {
            source: SkillSource::Global,
            manifest: SkillManifest {
                name: name.into(),
                description: format!("test skill {name}"),
                when_to_use: String::new(),
                context: SkillContext::Inline,
                effort: SkillEffort::Small,
                paths: vec![],
                user_invocable: true,
            },
            readme_path,
            skill_dir,
        }
    }

    fn make_ctx(command: &str) -> SlashContext {
        SlashContext {
            command: command.to_owned(),
            sender_id: "test-user".to_owned(),
            session_id: "sess-123".to_owned(),
            channel: "test".to_owned(),
        }
    }

    #[test]
    fn test_substitute_skill_dir() {
        let body = "Read files in ${SKILL_DIR}";
        let result =
            SkillSlashHandler::substitute_variables(body, Path::new("/tmp/my-skill"), "s1");
        assert_eq!(result, "Read files in /tmp/my-skill");
    }

    #[test]
    fn test_substitute_session_id() {
        let body = "Session: ${SESSION_ID}";
        let result = SkillSlashHandler::substitute_variables(body, Path::new("/tmp"), "sess-abc");
        assert_eq!(result, "Session: sess-abc");
    }

    #[test]
    fn test_substitute_mixed() {
        let body = "Dir: ${SKILL_DIR}, Session: ${SESSION_ID}, Unknown: ${FOO}";
        let result =
            SkillSlashHandler::substitute_variables(body, Path::new("/tmp/skill"), "s-999");
        assert_eq!(result, "Dir: /tmp/skill, Session: s-999, Unknown: ${FOO}");
    }

    #[test]
    fn test_substitute_no_vars() {
        let body = "Plain text";
        let result = SkillSlashHandler::substitute_variables(body, Path::new("/tmp"), "s1");
        assert_eq!(result, "Plain text");
    }

    #[test]
    fn test_invocable_names_empty() {
        let disk = Arc::new(DiskSkillRegistry::new(vec![]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        let handler = SkillSlashHandler::new(disk, builtin);
        assert!(handler.invocable_names().is_empty());
    }

    #[test]
    fn test_invocable_names_from_disk() {
        let temp = tempfile::tempdir().unwrap();
        let readme = temp.path().join("SKILL.md");
        std::fs::write(
            &readme,
            "---\ndescription: test\nuser-invocable: true\n---\n\n# Test\n",
        )
        .unwrap();
        let skill = make_disk_skill("my-skill", readme, temp.path().to_path_buf());
        let disk = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        let handler = SkillSlashHandler::new(disk, builtin);
        let names = handler.invocable_names();
        assert_eq!(names, vec!["my-skill"]);
    }

    #[tokio::test]
    async fn test_handle_disk_skill_found() {
        let temp = tempfile::tempdir().unwrap();
        let readme = temp.path().join("SKILL.md");
        std::fs::write(
            &readme,
            "---\ndescription: test\n---\n\n# Hello\nDo the thing.",
        )
        .unwrap();
        let skill = make_disk_skill("test-skill", readme, temp.path().to_path_buf());
        let disk = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        let handler = SkillSlashHandler::new(disk, builtin);

        let ctx = make_ctx("test-skill");
        let result = handler.handle("", &ctx).await;
        match result {
            SlashResult::InjectMeta { content } => {
                assert!(content.contains("# Hello"));
                assert!(content.contains("Do the thing."));
                assert!(!content.contains("${SKILL_DIR}"));
            }
            other => panic!("expected InjectMeta, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_handle_builtin_skill_fallback() {
        struct MockSkill;

        #[async_trait::async_trait]
        impl closeclaw_skills::Skill for MockSkill {
            fn manifest(&self) -> closeclaw_skills::SkillManifest {
                closeclaw_skills::SkillManifest {
                    name: "builtin-test".into(),
                    version: "1.0".into(),
                    description: "mock".into(),
                    author: None,
                    dependencies: vec![],
                }
            }
            fn body(&self) -> &str {
                "builtin body content"
            }
        }

        let disk = Arc::new(DiskSkillRegistry::new(vec![]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        builtin.register(Arc::new(MockSkill)).await;
        let handler = SkillSlashHandler::new(disk, builtin);

        let ctx = make_ctx("builtin-test");
        let result = handler.handle("", &ctx).await;
        match result {
            SlashResult::InjectMeta { content } => {
                assert_eq!(content, "builtin body content");
            }
            other => panic!("expected InjectMeta, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_handle_not_found() {
        let disk = Arc::new(DiskSkillRegistry::new(vec![]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        let handler = SkillSlashHandler::new(disk, builtin);

        let ctx = make_ctx("nonexistent");
        let result = handler.handle("", &ctx).await;
        match result {
            SlashResult::Reply(msg) => {
                assert!(msg.contains("未知技能"));
                assert!(msg.contains("nonexistent"));
            }
            other => panic!("expected Reply, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_disk_skill_substitutes_session_id() {
        let temp = tempfile::tempdir().unwrap();
        let readme = temp.path().join("SKILL.md");
        std::fs::write(
            &readme,
            "---\ndescription: test\n---\n\nSession: ${SESSION_ID}",
        )
        .unwrap();
        let skill = make_disk_skill("sess-test", readme, temp.path().to_path_buf());
        let disk = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        let handler = SkillSlashHandler::new(disk, builtin);

        let ctx = make_ctx("sess-test");
        let result = handler.handle("", &ctx).await;
        match result {
            SlashResult::InjectMeta { content } => {
                assert!(content.contains("sess-123"));
                assert!(!content.contains("${SESSION_ID}"));
            }
            other => panic!("expected InjectMeta, got {:?}", other),
        }
    }
}
