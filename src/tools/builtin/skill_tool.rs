//! Built-in tool — SkillTool
//!
//! Invokes a disk-based skill by looking it up in the [`DiskSkillRegistry`],
//! reading its SKILL.md file, and returning the content as a meta message
//! to be injected into the agent context.

use crate::agent::spawn::SpawnController;
use crate::gateway::session_manager::{SessionManager, SpawnMode};
use crate::skills::disk::types::SkillContext;
use crate::skills::disk::DiskSkillRegistry;
use crate::tools::{
    ContextModifier, Tool, ToolCallError, ToolContext, ToolFlags, ToolMessage, ToolResult,
};

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// SkillTool
// ---------------------------------------------------------------------------

/// Tool that loads and executes a disk-based skill.
///
/// When called, `SkillTool` looks up the named skill in the
/// [`DiskSkillRegistry`], and depending on the skill's context:
///
/// - **Inline / Agent**: injects `skill.body` as a meta message into the
///   agent context, with `context_modifier` for `allowed_tools`.
/// - **Fork**: creates an isolated child session via [`SessionManager`]
///   with `task = skill.body`.
pub struct SkillTool {
    registry: Arc<DiskSkillRegistry>,
    spawn_controller: Arc<SpawnController>,
    session_manager: Arc<SessionManager>,
}

impl SkillTool {
    /// Creates a new `SkillTool` backed by the given registry.
    pub fn new(
        registry: Arc<DiskSkillRegistry>,
        spawn_controller: Arc<SpawnController>,
        session_manager: Arc<SessionManager>,
    ) -> Self {
        Self {
            registry,
            spawn_controller,
            session_manager,
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
        "Load and execute a disk-based skill".to_string()
    }

    fn detail(&self) -> String {
        "Loads a skill definition from the disk-based skill registry and \
         makes it available to the agent. Call this tool with `skill_name` \
         (required) to retrieve the skill's SKILL.md content, which will be \
         injected as a meta message. The `args` field (optional) can pass \
         additional context to the skill."
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
            .ok_or_else(|| ToolCallError::InvalidArgs("skill_name is required".to_string()))?;

        // Look up the skill in the registry
        let skill = self
            .registry
            .get(skill_name)
            .ok_or_else(|| ToolCallError::NotFound(skill_name.to_string()))?;

        // Use skill.body (populated by loader) instead of reading from disk
        let body = skill.body.clone();

        // Build context_modifier from manifest.allowed_tools
        let context_modifier = if skill.manifest.allowed_tools.is_empty() {
            None
        } else {
            Some(ContextModifier {
                allowed_tools: skill.manifest.allowed_tools.clone(),
            })
        };

        // Dispatch based on SkillContext
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
                // Fork: create an isolated child session with skill.body as task
                let parent_session_id = ctx.session_id.as_deref().ok_or_else(|| {
                    ToolCallError::ExecutionFailed(
                        "no session_id in tool context (fork requires a tracked session)".into(),
                    )
                })?;

                // Validate spawn with the parent agent's config
                let config = self
                    .spawn_controller
                    .validate(parent_session_id, None)
                    .await
                    .map_err(|e| {
                        ToolCallError::ExecutionFailed(format!(
                            "fork spawn validation failed: {}",
                            e
                        ))
                    })?;

                // Get parent depth
                let parent_depth = self
                    .session_manager
                    .get_session_depth(parent_session_id)
                    .await
                    .unwrap_or(0);

                // Create child session with skill's allowed_tools whitelist.
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
                        false, // light_context
                        None,  // workspace
                        SpawnMode::Run,
                        false, // fork (parent history inheritance)
                        allowed_tools,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::spawn::SpawnController;
    use crate::config::ConfigManager;
    use crate::gateway::{GatewayConfig, SessionManager};
    use crate::session::bootstrap::loader::BootstrapMode;
    use crate::session::persistence::ReasoningLevel;
    use crate::skills::disk::types::{
        DiskSkill, SkillContext, SkillEffort, SkillManifest, SkillSource,
    };
    use std::sync::Arc;
    use tempfile::TempDir;

    fn test_deps() -> (Arc<SpawnController>, Arc<SessionManager>) {
        let tmp = TempDir::new().unwrap();
        let config_manager = Arc::new(
            ConfigManager::new(tmp.path().to_path_buf())
                .expect("ConfigManager::new should succeed"),
        );
        let session_manager = Arc::new(SessionManager::new(
            &GatewayConfig {
                name: "test".to_string(),
                rate_limit_per_minute: 100,
                max_message_size: 1024,
                dm_scope: crate::gateway::DmScope::default(),
            },
            None,
            None,
            BootstrapMode::Full,
            ReasoningLevel::default(),
        ));
        let spawn_controller = Arc::new(SpawnController::new(
            config_manager,
            session_manager.clone(),
        ));
        (spawn_controller, session_manager)
    }

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
        }
    }

    #[test]
    fn test_skill_tool_name() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (sc, sm) = test_deps();
        let tool = SkillTool::new(registry, sc, sm);
        assert_eq!(tool.name(), "SkillTool");
    }

    #[test]
    fn test_skill_tool_group() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (sc, sm) = test_deps();
        let tool = SkillTool::new(registry, sc, sm);
        assert_eq!(tool.group(), "skills");
    }

    #[test]
    fn test_skill_tool_summary_length() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (sc, sm) = test_deps();
        let tool = SkillTool::new(registry, sc, sm);
        let summary = tool.summary();
        assert!(
            summary.len() <= 50,
            "summary '{}' exceeds 50 chars",
            summary
        );
    }

    #[test]
    fn test_skill_tool_flags_is_deferred_false() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (sc, sm) = test_deps();
        let tool = SkillTool::new(registry, sc, sm);
        let flags = tool.flags();
        assert!(!flags.is_deferred_by_default);
    }

    #[test]
    fn test_skill_tool_input_schema_contains_skill_name() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (sc, sm) = test_deps();
        let tool = SkillTool::new(registry, sc, sm);
        let schema = tool.input_schema();
        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("skill_name"));
        assert!(props.contains_key("args"));
        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(required.contains(&serde_json::json!("skill_name")));
    }

    // -----------------------------------------------------------------
    // call() — error cases
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_call_skill_not_found() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (sc, sm) = test_deps();
        let tool = SkillTool::new(registry, sc, sm);
        let result = tool
            .call(serde_json::json!({"skill_name": "nonexistent"}), &new_ctx())
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ToolCallError::NotFound(_)));
        let ToolCallError::NotFound(name) = err else {
            unreachable!()
        };
        assert_eq!(name, "nonexistent");
    }

    #[tokio::test]
    async fn test_call_missing_skill_name() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (sc, sm) = test_deps();
        let tool = SkillTool::new(registry, sc, sm);
        let result = tool.call(serde_json::json!({}), &new_ctx()).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ToolCallError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn test_call_skill_name_wrong_type() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (sc, sm) = test_deps();
        let tool = SkillTool::new(registry, sc, sm);
        let result = tool
            .call(serde_json::json!({"skill_name": 123}), &new_ctx())
            .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ToolCallError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn test_call_readme_file_not_found() {
        // With body pre-populated by the loader, an empty body is used
        // even when readme_path doesn't exist (load_body is no longer called).
        let mut skill = make_skill(
            "orphan",
            vec![],
            std::path::PathBuf::from("/nonexistent/path/SKILL.md"),
        );
        skill.body = "".to_string();
        let registry = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let (sc, sm) = test_deps();
        let tool = SkillTool::new(registry, sc, sm);
        let result = tool
            .call(serde_json::json!({"skill_name": "orphan"}), &new_ctx())
            .await;
        // Should succeed with empty body (inline mode)
        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.data["execution_mode"], "inline");
    }

    #[tokio::test]
    async fn test_call_fork_mode_no_session() {
        // Fork without session_id in context should fail
        let temp = TempDir::new().unwrap();
        let readme_path = temp.path().join("SKILL.md");
        let skill_content = "---\ndescription: A fork skill\n---\n\n# Fork Skill\n";
        std::fs::write(&readme_path, skill_content).unwrap();

        let mut skill = make_skill("forkskill", vec![], readme_path);
        skill.body = "# Fork Skill".to_string();
        skill.manifest.context = SkillContext::Fork;
        let registry = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let (sc, sm) = test_deps();
        let tool = SkillTool::new(registry, sc, sm);

        // new_ctx() has session_id = None
        let result = tool
            .call(serde_json::json!({"skill_name": "forkskill"}), &new_ctx())
            .await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolCallError::ExecutionFailed(_)
        ));
    }
}
