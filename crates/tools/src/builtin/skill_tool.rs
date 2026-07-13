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
    spawn_validator: Arc<dyn SpawnValidator>,
    session_manager: Arc<SessionManager>,
}

impl SkillTool {
    /// Creates a new `SkillTool` backed by the given registry.
    pub fn new(
        registry: Arc<DiskSkillRegistry>,
        spawn_validator: Arc<dyn SpawnValidator>,
        session_manager: Arc<SessionManager>,
    ) -> Self {
        Self {
            registry,
            spawn_validator,
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
                        None, // model_override
                        None, // parent_subagents_model
                        1,    // max_spawn_depth (skill tool doesn't spawn)
                        None, // spawn_timeout (skill tool doesn't set timeout)
                        None, // label
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
    use crate::{SpawnError, SpawnValidationResult, SpawnValidator, ToolContext};
    use closeclaw_config::agents::MemoryConfig;
    use closeclaw_session::persistence::ReasoningLevel;
    use closeclaw_skills::disk::types::{
        DiskSkill, SkillContext, SkillEffort, SkillManifest, SkillSource,
    };
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
        let tool = SkillTool::new(
            registry,
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
        let tool = SkillTool::new(
            registry,
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
        let tool = SkillTool::new(
            registry,
            Arc::new(MockSpawnValidator),
            make_session_manager(),
        );
        let result = tool.call(serde_json::json!({}), &new_ctx()).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ToolCallError::InvalidArgs(_)));
    }
}
