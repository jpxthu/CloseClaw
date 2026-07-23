//! Built-in tool — SkillTool
//!
//! Invokes a disk-based skill by looking it up in the [`DiskSkillRegistry`],
//! reading its SKILL.md file, and returning the content as a meta message
//! to be injected into the agent context.

use crate::{
    ContextModifier, SpawnValidator, Tool, ToolCallError, ToolContext, ToolFlags, ToolMessage,
    ToolResult,
};
use closeclaw_gateway::session_manager::{SessionManager, SpawnMode};
use closeclaw_skills::disk::types::SkillContext;
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
/// - **Disk skill — Inline / Agent**: injects `skill.body` as a meta
///   message into the agent context, with `context_modifier` for
///   `allowed_tools`.
/// - **Disk skill — Fork**: creates an isolated child session via
///   [`SessionManager`] with `task = skill.body`.
/// - **Builtin skill**: calls `execute("invoke", args)` and injects the
///   result as a meta message (always Inline mode).
pub struct SkillTool {
    registry: Arc<DiskSkillRegistry>,
    builtin_registry: Arc<BuiltinSkillRegistry>,
    spawn_validator: Arc<dyn SpawnValidator>,
    session_manager: Arc<SessionManager>,
}

impl SkillTool {
    /// Creates a new `SkillTool` backed by the given registries.
    pub fn new(
        registry: Arc<DiskSkillRegistry>,
        builtin_registry: Arc<BuiltinSkillRegistry>,
        spawn_validator: Arc<dyn SpawnValidator>,
        session_manager: Arc<SessionManager>,
    ) -> Self {
        Self {
            registry,
            builtin_registry,
            spawn_validator,
            session_manager,
        }
    }

    /// Handle a disk-based skill lookup.
    async fn call_disk_skill(
        &self,
        skill_name: &str,
        skill: &closeclaw_skills::disk::types::DiskSkill,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolCallError> {
        let body = skill.body.clone();

        let context_modifier = if skill.manifest.allowed_tools.is_empty() {
            None
        } else {
            Some(ContextModifier {
                allowed_tools: skill.manifest.allowed_tools.clone(),
            })
        };

        match &skill.manifest.context {
            SkillContext::Inline => Ok(ToolResult {
                data: serde_json::json!({
                    "skill_name": skill_name,
                    "status": "loaded",
                    "execution_mode": "inline"
                }),
                new_messages: vec![ToolMessage {
                    content: body,
                    is_meta: true,
                }],
                context_modifier,
            }),
            SkillContext::Agent { agent_id } => Ok(ToolResult {
                data: serde_json::json!({
                    "skill_name": skill_name,
                    "status": "loaded",
                    "execution_mode": "agent",
                    "agent_id": agent_id
                }),
                new_messages: vec![ToolMessage {
                    content: body,
                    is_meta: true,
                }],
                context_modifier,
            }),
            SkillContext::Fork => {
                let parent_session_id = ctx.session_id.as_deref().ok_or_else(|| {
                    ToolCallError::ExecutionFailed(
                        "no session_id in tool context (fork requires a tracked session)".into(),
                    )
                })?;

                let spawn_result = self
                    .spawn_validator
                    .validate_spawn(parent_session_id, None)
                    .await
                    .map_err(|e| {
                        ToolCallError::ExecutionFailed(format!(
                            "fork spawn validation failed: {}",
                            e
                        ))
                    })?;
                let config = spawn_result.config;

                let parent_depth = self
                    .session_manager
                    .get_session_depth(parent_session_id)
                    .await
                    .unwrap_or(0);

                let allowed_tools = if skill.manifest.allowed_tools.is_empty() {
                    None
                } else {
                    Some(skill.manifest.allowed_tools.clone())
                };
                let child_session_id = self
                    .session_manager
                    .create_child_session(
                        &config,
                        parent_session_id,
                        parent_depth + 1,
                        &body,
                        false,
                        None,
                        SpawnMode::Run,
                        false,
                        allowed_tools,
                        None,
                        None,
                        1,
                        None,
                        None,
                        None,
                    )
                    .await
                    .map_err(|e| {
                        ToolCallError::ExecutionFailed(format!(
                            "fork child session creation failed: {}",
                            e
                        ))
                    })?;

                Ok(ToolResult {
                    data: serde_json::json!({
                        "skill_name": skill_name,
                        "status": "spawned",
                        "execution_mode": "fork",
                        "child_session_id": child_session_id
                    }),
                    new_messages: vec![],
                    context_modifier,
                })
            }
        }
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
    use crate::{SpawnError, SpawnValidationResult, SpawnValidator, ToolContext};
    use closeclaw_common::BootstrapMode;
    use closeclaw_config::agents::MemoryConfig;
    use closeclaw_session::persistence::ReasoningLevel;
    use closeclaw_skills::disk::types::{
        DiskSkill, SkillContext, SkillEffort, SkillManifest, SkillSource,
    };
    use closeclaw_skills::BuiltinSkillRegistry;
    use std::sync::Arc;

    struct MockSpawnValidator;

    #[async_trait::async_trait]
    impl SpawnValidator for MockSpawnValidator {
        async fn validate_spawn(
            &self,
            _parent_session_id: &str,
            _target_agent_id: Option<&str>,
        ) -> Result<SpawnValidationResult, SpawnError> {
            Ok(SpawnValidationResult {
                config: closeclaw_config::agents::ResolvedAgentConfig {
                    id: "mock-agent".to_string(),
                    name: "mock-agent".to_string(),
                    parent_id: None,
                    model: None,
                    workspace: None,
                    agent_dir: None,
                    bootstrap_mode: BootstrapMode::Full,
                    skills: vec![],
                    tools: vec![],
                    disallowed_tools: vec![],
                    subagents: Default::default(),
                    memory: MemoryConfig::default(),
                    hooks: Vec::new(),
                    source: closeclaw_config::agents::ConfigSource::Merged,
                },
                effective_max_spawn_depth: 10,
                spawn_timeout: None,
            })
        }
    }

    fn make_session_manager() -> Arc<closeclaw_gateway::SessionManager> {
        Arc::new(closeclaw_gateway::SessionManager::new(
            &closeclaw_gateway::GatewayConfig {
                name: "test".to_string(),
                rate_limit_per_minute: 100,
                max_message_size: 1024,
                ..Default::default()
            },
            None,
            None,
            ReasoningLevel::default(),
        ))
    }

    #[allow(dead_code)]
    fn make_skill(
        name: &str,
        allowed_tools: Vec<String>,
        readme_path: std::path::PathBuf,
    ) -> DiskSkill {
        DiskSkill {
            source: SkillSource::Bundled,
            manifest: SkillManifest {
                name: name.into(),
                description: format!("A test skill named {}", name),
                allowed_tools,
                when_to_use: String::new(),
                context: SkillContext::Inline,
                agent: String::new(),
                agent_id: String::new(),
                effort: SkillEffort::Small,
                paths: vec![],
                user_invocable: false,
            },
            readme_path,
            skill_dir: std::path::PathBuf::new(),
            body: String::new(),
        }
    }

    fn new_ctx() -> ToolContext {
        ToolContext {
            agent_id: "test-agent".to_string(),
            workdir: None,
            session_id: None,
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
        let tool = SkillTool::new(
            registry,
            builtin,
            Arc::new(MockSpawnValidator),
            make_session_manager(),
        );
        assert_eq!(tool.name(), "SkillTool");
    }

    // -----------------------------------------------------------------
    // call() — error cases
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_call_skill_not_found() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        let tool = SkillTool::new(
            registry,
            builtin,
            Arc::new(MockSpawnValidator),
            make_session_manager(),
        );
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
        let tool = SkillTool::new(
            registry,
            builtin,
            Arc::new(MockSpawnValidator),
            make_session_manager(),
        );
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
        let tool = SkillTool::new(
            disk,
            builtin,
            Arc::new(MockSpawnValidator),
            make_session_manager(),
        );
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
        let disk_skill = make_skill("shared", vec![], std::path::PathBuf::from("/tmp/test"));
        let disk = Arc::new(DiskSkillRegistry::new(vec![disk_skill]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        builtin
            .register(Arc::new(MockBuiltinSkill("shared".into())))
            .await;
        let tool = SkillTool::new(
            disk,
            builtin,
            Arc::new(MockSpawnValidator),
            make_session_manager(),
        );
        let result = tool
            .call(serde_json::json!({"skill_name": "shared"}), &new_ctx())
            .await
            .unwrap();
        // Disk skill returns execution_mode "inline" from SkillContext::Inline
        assert_eq!(result.data["execution_mode"], "inline");
        // Disk skill has no allowed_tools → no context_modifier
        assert!(result.context_modifier.is_none());
    }
}
