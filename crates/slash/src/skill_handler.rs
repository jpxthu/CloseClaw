//! Slash handler for user-invocable skills.
//!
//! Routes `/<skill-name>` to the matching skill in the disk or builtin
//! registry, loads its body, and injects it into the agent context
//! via [`SlashResult::SystemAppend`].

use std::sync::Arc;

use closeclaw_skills::disk::DiskSkillRegistry;
use closeclaw_skills::BuiltinSkillRegistry;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use closeclaw_common::slash_router::{SlashResult, SystemAppendAction};

/// Handler that dispatches `/<skill-name>` to the matching skill.
///
/// The command names are **not** declared statically — they are injected
/// at registration time via [`HandlerRegistry::register_named`]. This
/// allows the handler to dynamically claim any skill name that appears
/// in the disk or builtin registries.
#[derive(Clone)]
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
    pub async fn invocable_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.disk_registry.user_invocable_names();
        let builtin_names = self.builtin_registry.user_invocable_names().await;
        names.extend(builtin_names);
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
            return SlashResult::SystemAppend {
                action: SystemAppendAction::Add(body),
            };
        }

        // 2. Fallback: BuiltinSkillRegistry
        if let Some(skill) = self.builtin_registry.get(skill_name).await {
            return match skill.execute(None).await {
                Ok(content) => SlashResult::SystemAppend {
                    action: SystemAppendAction::Add(content),
                },
                Err(e) => SlashResult::Reply(format!("技能 \"{skill_name}\" 执行失败: {e}")),
            };
        }

        // 3. Not found
        SlashResult::Reply(format!("未知技能: {skill_name}"))
    }

    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(self.clone())
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

    #[tokio::test]
    async fn test_invocable_names_empty() {
        let disk = Arc::new(DiskSkillRegistry::new(vec![]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        let handler = SkillSlashHandler::new(disk, builtin);
        assert!(handler.invocable_names().await.is_empty());
    }

    #[tokio::test]
    async fn test_invocable_names_from_disk() {
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
        let names = handler.invocable_names().await;
        assert_eq!(names, vec!["my-skill"]);
    }

    #[tokio::test]
    async fn test_invocable_names_includes_builtin() {
        struct BuiltinMockSkill {
            invocable: bool,
        }

        #[async_trait::async_trait]
        impl closeclaw_skills::Skill for BuiltinMockSkill {
            fn manifest(&self) -> closeclaw_skills::SkillManifest {
                closeclaw_skills::SkillManifest {
                    name: "builtin-skill".into(),
                    version: "1.0".into(),
                    description: "mock".into(),
                    author: None,
                    dependencies: vec![],
                }
            }
            fn body(&self) -> &str {
                "builtin body"
            }
            fn listing_meta(&self) -> closeclaw_skills::SkillListingMeta {
                closeclaw_skills::SkillListingMeta {
                    user_invocable: self.invocable,
                    ..Default::default()
                }
            }
        }

        // Disk skill + builtin skill, both invocable
        let temp = tempfile::tempdir().unwrap();
        let readme = temp.path().join("SKILL.md");
        std::fs::write(
            &readme,
            "---\ndescription: test\nuser-invocable: true\n---\n\n# Test\n",
        )
        .unwrap();
        let skill = make_disk_skill("disk-skill", readme, temp.path().to_path_buf());
        let disk = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        builtin
            .register(Arc::new(BuiltinMockSkill { invocable: true }))
            .await;
        let handler = SkillSlashHandler::new(disk, builtin);
        let mut names = handler.invocable_names().await;
        names.sort();
        assert_eq!(names, vec!["builtin-skill", "disk-skill"]);
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
            SlashResult::SystemAppend {
                action: SystemAppendAction::Add(content),
            } => {
                assert!(content.contains("# Hello"));
                assert!(content.contains("Do the thing."));
                assert!(!content.contains("${SKILL_DIR}"));
            }
            other => panic!("expected SystemAppend(Add), got {:?}", other),
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
            SlashResult::SystemAppend {
                action: SystemAppendAction::Add(content),
            } => {
                assert_eq!(content, "builtin body content");
            }
            other => panic!("expected SystemAppend(Add), got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_handle_builtin_skill_uses_execute_not_body() {
        struct ExecuteOverrideSkill;

        #[async_trait::async_trait]
        impl closeclaw_skills::Skill for ExecuteOverrideSkill {
            fn manifest(&self) -> closeclaw_skills::SkillManifest {
                closeclaw_skills::SkillManifest {
                    name: "exec-override".into(),
                    version: "1.0".into(),
                    description: "mock".into(),
                    author: None,
                    dependencies: vec![],
                }
            }
            fn body(&self) -> &str {
                "this is body text"
            }
            async fn execute(
                &self,
                _args: Option<serde_json::Value>,
            ) -> Result<String, closeclaw_skills::SkillError> {
                Ok("execute result".to_string())
            }
        }

        let disk = Arc::new(DiskSkillRegistry::new(vec![]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        builtin.register(Arc::new(ExecuteOverrideSkill)).await;
        let handler = SkillSlashHandler::new(disk, builtin);

        let ctx = make_ctx("exec-override");
        let result = handler.handle("", &ctx).await;
        match result {
            SlashResult::SystemAppend {
                action: SystemAppendAction::Add(content),
            } => {
                assert_eq!(content, "execute result");
                assert_ne!(content, "this is body text");
            }
            other => panic!("expected SystemAppend(Add), got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_handle_builtin_skill_execute_error() {
        struct ErrorSkill;

        #[async_trait::async_trait]
        impl closeclaw_skills::Skill for ErrorSkill {
            fn manifest(&self) -> closeclaw_skills::SkillManifest {
                closeclaw_skills::SkillManifest {
                    name: "error-skill".into(),
                    version: "1.0".into(),
                    description: "mock".into(),
                    author: None,
                    dependencies: vec![],
                }
            }
            fn body(&self) -> &str {
                "error body"
            }
            async fn execute(
                &self,
                _args: Option<serde_json::Value>,
            ) -> Result<String, closeclaw_skills::SkillError> {
                Err(closeclaw_skills::SkillError::ExecutionFailed(
                    "boom".to_string(),
                ))
            }
        }

        let disk = Arc::new(DiskSkillRegistry::new(vec![]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        builtin.register(Arc::new(ErrorSkill)).await;
        let handler = SkillSlashHandler::new(disk, builtin);

        let ctx = make_ctx("error-skill");
        let result = handler.handle("", &ctx).await;
        match result {
            SlashResult::Reply(msg) => {
                assert!(msg.contains("执行失败"));
                assert!(msg.contains("boom"));
            }
            other => panic!("expected Reply, got {:?}", other),
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
            SlashResult::SystemAppend {
                action: SystemAppendAction::Add(content),
            } => {
                assert!(content.contains("sess-123"));
                assert!(!content.contains("${SESSION_ID}"));
            }
            other => panic!("expected SystemAppend(Add), got {:?}", other),
        }
    }

    // ── Error-path tests for SystemAppend injection ──────────────────────

    /// When a disk skill's SKILL.md exists but has empty body (only frontmatter),
    /// the handler should still return SystemAppend with the empty string —
    /// the executor-side handles empty content gracefully.
    #[tokio::test]
    async fn test_disk_skill_empty_body_returns_system_append() {
        let temp = tempfile::tempdir().unwrap();
        let readme = temp.path().join("SKILL.md");
        std::fs::write(&readme, "---\ndescription: test\n---\n").unwrap();
        let skill = make_disk_skill("empty-skill", readme, temp.path().to_path_buf());
        let disk = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        let handler = SkillSlashHandler::new(disk, builtin);

        let ctx = make_ctx("empty-skill");
        let result = handler.handle("", &ctx).await;
        match result {
            SlashResult::SystemAppend {
                action: SystemAppendAction::Add(content),
            } => {
                // Empty body is still injected — no special-casing at handler level.
                assert!(
                    content.is_empty(),
                    "expected empty content, got: {:?}",
                    content
                );
            }
            other => panic!("expected SystemAppend(Add) for empty body, got {:?}", other),
        }
    }

    /// When a disk skill's load_body() fails (e.g. file deleted after registration),
    /// the handler returns a Reply error — it does NOT fall through to SystemAppend.
    #[tokio::test]
    async fn test_disk_skill_load_failure_returns_error_reply() {
        let temp = tempfile::tempdir().unwrap();
        let readme = temp.path().join("SKILL.md");
        std::fs::write(&readme, "---\ndescription: test\n---\n\n# Skill Body").unwrap();
        let skill = make_disk_skill("failing-skill", readme.clone(), temp.path().to_path_buf());
        let disk = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        let handler = SkillSlashHandler::new(disk, builtin);

        // Delete the file after registration to simulate load failure.
        std::fs::remove_file(&readme).unwrap();

        let ctx = make_ctx("failing-skill");
        let result = handler.handle("", &ctx).await;
        match result {
            SlashResult::Reply(msg) => {
                assert!(
                    msg.contains("加载失败"),
                    "should report load failure, got: {msg}"
                );
                assert!(
                    msg.contains("failing-skill"),
                    "should include skill name, got: {msg}"
                );
            }
            other => panic!("expected Reply (load failure), got {:?}", other),
        }
    }

    /// End-to-end: skill body content is injected verbatim into SystemAppend::Add.
    /// This verifies the equivalent behavior of the old InjectMeta path — the skill
    /// content is passed through without transformation (beyond variable substitution).
    #[tokio::test]
    async fn test_skill_content_injected_verbatim_as_system_append() {
        let temp = tempfile::tempdir().unwrap();
        let readme = temp.path().join("SKILL.md");
        let body = "---\ndescription: verbatim test\n---\n\n# Instructions\nBe concise.\n\nRules:\n1. No hallucination\n2. Cite sources";
        std::fs::write(&readme, body).unwrap();
        let skill = make_disk_skill("verbatim-skill", readme, temp.path().to_path_buf());
        let disk = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        let handler = SkillSlashHandler::new(disk, builtin);

        let ctx = make_ctx("verbatim-skill");
        let result = handler.handle("", &ctx).await;
        match result {
            SlashResult::SystemAppend {
                action: SystemAppendAction::Add(content),
            } => {
                // The skill body (minus frontmatter, as loaded by load_body) should
                // appear in the SystemAppend content.
                assert!(
                    content.contains("# Instructions"),
                    "missing heading, got: {content}"
                );
                assert!(
                    content.contains("No hallucination"),
                    "missing rule, got: {content}"
                );
                assert!(
                    content.contains("Cite sources"),
                    "missing rule, got: {content}"
                );
                // Frontmatter should NOT be in the body (stripped by load_body).
                assert!(
                    !content.contains("description: verbatim test"),
                    "frontmatter leaked, got: {content}"
                );
            }
            other => panic!(
                "expected SystemAppend(Add) with body content, got {:?}",
                other
            ),
        }
    }
}
