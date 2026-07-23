//! Built-in tool — SkillTool
//!
//! Invokes a disk-based skill by looking it up in the [`DiskSkillRegistry`],
//! reading its SKILL.md file, and returning the content as a meta message
//! to be injected into the agent context.

use crate::{Tool, ToolCallError, ToolContext, ToolFlags, ToolMessage, ToolResult};
use closeclaw_skills::disk::DiskSkillRegistry;
use closeclaw_skills::BuiltinSkillRegistry;

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// SkillTool
// ---------------------------------------------------------------------------

/// Tool that loads and executes a disk-based or builtin skill.
///
/// When called, `SkillTool` first looks up the named skill in the
/// [`DiskSkillRegistry`]. If not found, it falls back to the
/// [`BuiltinSkillRegistry`].
///
/// - **Disk skill**: injects the skill body (loaded via `load_body()`) as a meta message into the
///   agent context.
/// - **Builtin skill**: calls `execute("invoke", args)` and injects the
///   result as a meta message.
pub struct SkillTool {
    registry: Arc<DiskSkillRegistry>,
    builtin_registry: Arc<BuiltinSkillRegistry>,
}

impl SkillTool {
    /// Creates a new `SkillTool` backed by the given registries.
    pub fn new(
        registry: Arc<DiskSkillRegistry>,
        builtin_registry: Arc<BuiltinSkillRegistry>,
    ) -> Self {
        Self {
            registry,
            builtin_registry,
        }
    }

    /// Handle a disk-based skill lookup.
    ///
    /// Injects the skill body as a meta message into the agent context,
    /// after substituting `${SKILL_DIR}` and `${SESSION_ID}` variables.
    async fn call_disk_skill(
        &self,
        skill_name: &str,
        skill: &closeclaw_skills::disk::types::DiskSkill,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolCallError> {
        let body_str = skill.load_body().map_err(|e| {
            ToolCallError::ExecutionFailed(format!("failed to load skill body: {}", e))
        })?;
        let body = Self::substitute_variables(&body_str, skill, ctx);

        Ok(ToolResult {
            data: serde_json::json!({
                "skill_name": skill_name,
                "status": "loaded",
                "execution_mode": "inline"
            }),
            new_messages: vec![ToolMessage {
                content: body,
                is_meta: true,
            }],
            context_modifier: None,
        })
    }

    /// Replace `${SKILL_DIR}` and `${SESSION_ID}` placeholders in the skill body.
    ///
    /// - `${SKILL_DIR}` → absolute path to the skill directory
    /// - `${SESSION_ID}` → current session ID from ToolContext
    /// - Unrecognized `${...}` patterns remain unchanged
    fn substitute_variables(
        body: &str,
        skill: &closeclaw_skills::disk::types::DiskSkill,
        ctx: &ToolContext,
    ) -> String {
        let mut result = body.to_string();

        let skill_dir_str = skill.skill_dir.to_string_lossy().to_string();
        result = result.replace("${SKILL_DIR}", &skill_dir_str);

        if let Some(ref session_id) = ctx.session_id {
            result = result.replace("${SESSION_ID}", session_id);
        }

        result
    }

    /// Handle a builtin skill lookup.
    ///
    /// Calls `execute("invoke", args)` and wraps the result as a
    /// meta-message in Inline mode.
    async fn call_builtin_skill(
        &self,
        skill_name: &str,
        skill: Arc<dyn closeclaw_skills::Skill>,
        args: Value,
    ) -> Result<ToolResult, ToolCallError> {
        let result = skill.execute("invoke", args).await;
        match result {
            Ok(value) => Ok(ToolResult {
                data: serde_json::json!({
                    "skill_name": skill_name,
                    "status": "loaded",
                    "execution_mode": "inline"
                }),
                new_messages: vec![ToolMessage {
                    content: serde_json::to_string(&value).unwrap_or_else(|_| value.to_string()),
                    is_meta: true,
                }],
                context_modifier: None,
            }),
            Err(e) => Err(ToolCallError::ExecutionFailed(format!(
                "builtin skill '{}' execution failed: {}",
                skill_name, e
            ))),
        }
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "SkillTool"
    }

    fn group(&self) -> &str {
        "skills"
    }

    fn summary(&self) -> String {
        "Load and execute a disk-based or builtin skill".to_string()
    }

    fn detail(&self) -> String {
        "Loads a skill via unified routing: first checks the disk-based \
         skill registry, then falls back to the builtin skill registry. \
         Call this tool with `skill_name` (required) to retrieve the \
         skill's content, which will be injected as a meta message. The \
         `args` field (optional) can pass additional context to the skill."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "skill_name": {
                    "type": "string",
                    "description": "Name of the skill to load (e.g. 'clawhub')"
                },
                "args": {
                    "type": "object",
                    "description": "Optional arguments to pass to the skill"
                }
            },
            "required": ["skill_name"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_deferred_by_default: false,
            ..Default::default()
        }
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        // Extract skill_name from args
        let skill_name = args
            .get("skill_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolCallError::InvalidArgs("skill_name is required".to_string()))?
            .to_string();

        // --- Unified routing: Disk first, Builtin fallback ---
        if let Some(skill) = self.registry.get(&skill_name) {
            return self.call_disk_skill(&skill_name, skill, ctx).await;
        }

        // Fallback: Builtin skill registry
        if let Some(skill) = self.builtin_registry.get(&skill_name).await {
            return self.call_builtin_skill(&skill_name, skill, args).await;
        }

        Err(ToolCallError::NotFound(skill_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolContext;
    use closeclaw_skills::disk::types::{
        DiskSkill, SkillContext, SkillEffort, SkillManifest, SkillSource,
    };
    use closeclaw_skills::BuiltinSkillRegistry;
    use std::sync::Arc;

    #[allow(dead_code)]
    fn make_skill(name: &str, readme_path: std::path::PathBuf) -> DiskSkill {
        DiskSkill {
            source: SkillSource::Bundled,
            manifest: SkillManifest {
                name: name.into(),
                description: format!("A test skill named {}", name),
                when_to_use: String::new(),
                context: SkillContext::Inline,
                effort: SkillEffort::Small,
                paths: vec![],
                user_invocable: false,
            },
            readme_path,
            skill_dir: std::path::PathBuf::new(),
        }
    }

    fn make_skill_with_body(name: &str, _body: &str, skill_dir: std::path::PathBuf) -> DiskSkill {
        DiskSkill {
            source: SkillSource::Bundled,
            manifest: SkillManifest {
                name: name.into(),
                description: format!("A test skill named {}", name),
                when_to_use: String::new(),
                context: SkillContext::Inline,
                effort: SkillEffort::Small,
                paths: vec![],
                user_invocable: false,
            },
            readme_path: std::path::PathBuf::new(),
            skill_dir,
        }
    }

    fn new_ctx() -> ToolContext {
        new_ctx_with_session(None)
    }

    fn new_ctx_with_session(session_id: Option<String>) -> ToolContext {
        ToolContext {
            agent_id: "test-agent".to_string(),
            workdir: None,
            session_id,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        }
    }

    #[test]
    fn test_skill_tool_name() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        let tool = SkillTool::new(registry, builtin);
        assert_eq!(tool.name(), "SkillTool");
    }

    // -----------------------------------------------------------------
    // call() — error cases
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_call_skill_not_found() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        let tool = SkillTool::new(registry, builtin);
        let result = tool
            .call(serde_json::json!({"skill_name": "nonexistent"}), &new_ctx())
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ToolCallError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_call_missing_skill_name() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        let tool = SkillTool::new(registry, builtin);
        let result = tool.call(serde_json::json!({}), &new_ctx()).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ToolCallError::InvalidArgs(_)));
    }

    // -----------------------------------------------------------------
    // Unified routing — builtin fallback
    // -----------------------------------------------------------------

    struct MockBuiltinSkill(String);

    #[async_trait::async_trait]
    impl closeclaw_skills::Skill for MockBuiltinSkill {
        fn manifest(&self) -> closeclaw_skills::SkillManifest {
            closeclaw_skills::SkillManifest {
                name: self.0.clone(),
                version: "1.0".into(),
                description: "mock builtin".into(),
                author: None,
                dependencies: vec![],
            }
        }
        fn methods(&self) -> Vec<&str> {
            vec!["invoke"]
        }
        async fn execute(
            &self,
            _method: &str,
            _args: serde_json::Value,
        ) -> Result<serde_json::Value, closeclaw_skills::SkillError> {
            Ok(serde_json::json!({"output": "builtin result"}))
        }
    }

    #[tokio::test]
    async fn test_call_builtin_skill_fallback() {
        let disk = Arc::new(DiskSkillRegistry::new(vec![]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        builtin
            .register(Arc::new(MockBuiltinSkill("my_builtin".into())))
            .await;
        let tool = SkillTool::new(disk, builtin);
        let result = tool
            .call(serde_json::json!({"skill_name": "my_builtin"}), &new_ctx())
            .await
            .unwrap();
        assert_eq!(result.data["skill_name"], "my_builtin");
        assert_eq!(result.data["status"], "loaded");
        assert_eq!(result.data["execution_mode"], "inline");
        assert_eq!(result.new_messages.len(), 1);
        assert!(result.new_messages[0].is_meta);
        assert!(result.new_messages[0].content.contains("builtin result"));
        // Builtin skills never have a context_modifier
        assert!(result.context_modifier.is_none());
    }

    #[tokio::test]
    async fn test_call_disk_priority_over_builtin() {
        let disk_skill = make_skill("shared", std::path::PathBuf::from("/tmp/test"));
        let disk = Arc::new(DiskSkillRegistry::new(vec![disk_skill]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        builtin
            .register(Arc::new(MockBuiltinSkill("shared".into())))
            .await;
        let tool = SkillTool::new(disk, builtin);
        let result = tool
            .call(serde_json::json!({"skill_name": "shared"}), &new_ctx())
            .await
            .unwrap();
        // Disk skill returns execution_mode "inline" from SkillContext::Inline
        assert_eq!(result.data["execution_mode"], "inline");
        // Disk skill has no context_modifier (skills don't carry tool permissions)
        assert!(result.context_modifier.is_none());
    }

    // -----------------------------------------------------------------
    // Variable substitution tests
    // -----------------------------------------------------------------

    #[test]
    fn test_substitute_skill_dir() {
        let skill = make_skill_with_body(
            "test",
            "Read files in ${SKILL_DIR}",
            std::path::PathBuf::from("/home/user/.closeclaw/skills/my-skill"),
        );
        let ctx = new_ctx();
        let body_str = "Read files in ${SKILL_DIR}";
        let result = SkillTool::substitute_variables(body_str, &skill, &ctx);
        assert_eq!(
            result,
            "Read files in /home/user/.closeclaw/skills/my-skill"
        );
    }

    #[test]
    fn test_substitute_session_id() {
        let skill = make_skill_with_body(
            "test",
            "Session: ${SESSION_ID}",
            std::path::PathBuf::from("/tmp/skill"),
        );
        let ctx = new_ctx_with_session(Some("sess-abc-123".to_string()));
        let body_str = "Session: ${SESSION_ID}";
        let result = SkillTool::substitute_variables(body_str, &skill, &ctx);
        assert_eq!(result, "Session: sess-abc-123");
    }

    #[test]
    fn test_substitute_unknown_variable_preserved() {
        let skill = make_skill_with_body(
            "test",
            "Hello ${UNKNOWN_VAR}",
            std::path::PathBuf::from("/tmp/skill"),
        );
        let ctx = new_ctx();
        let body_str = "Hello ${UNKNOWN_VAR}";
        let result = SkillTool::substitute_variables(body_str, &skill, &ctx);
        assert_eq!(result, "Hello ${UNKNOWN_VAR}");
    }

    #[test]
    fn test_substitute_no_variables() {
        let skill = make_skill_with_body(
            "test",
            "Plain text without variables",
            std::path::PathBuf::from("/tmp/skill"),
        );
        let ctx = new_ctx();
        let body_str = "Plain text without variables";
        let result = SkillTool::substitute_variables(body_str, &skill, &ctx);
        assert_eq!(result, "Plain text without variables");
    }

    #[test]
    fn test_substitute_mixed_known_and_unknown() {
        let skill = make_skill_with_body(
            "test",
            "Dir: ${SKILL_DIR}, Session: ${SESSION_ID}, Unknown: ${FOO}",
            std::path::PathBuf::from("/tmp/my-skill"),
        );
        let ctx = new_ctx_with_session(Some("s-999".to_string()));
        let body_str = "Dir: ${SKILL_DIR}, Session: ${SESSION_ID}, Unknown: ${FOO}";
        let result = SkillTool::substitute_variables(body_str, &skill, &ctx);
        assert_eq!(
            result,
            "Dir: /tmp/my-skill, Session: s-999, Unknown: ${FOO}"
        );
    }

    #[test]
    fn test_substitute_session_id_none() {
        let skill = make_skill_with_body(
            "test",
            "Session: ${SESSION_ID}",
            std::path::PathBuf::from("/tmp/skill"),
        );
        let ctx = new_ctx_with_session(None);
        let body_str = "Session: ${SESSION_ID}";
        let result = SkillTool::substitute_variables(body_str, &skill, &ctx);
        // session_id is None → placeholder remains unchanged
        assert_eq!(result, "Session: ${SESSION_ID}");
    }
}
