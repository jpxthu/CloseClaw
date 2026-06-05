//! Built-in sessions_spawn tool — creates child sessions for sub-agents.

use crate::agent::spawn::SpawnController;
use crate::gateway::session_manager::{SessionManager, SpawnMode};
use crate::tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Tool that spawns child sessions for sub-agent execution.
///
/// Holds `Arc` references to [`SpawnController`] (for spawn-time validation)
/// and [`SessionManager`] (for depth lookup and child session creation),
/// following the same constructor-injection pattern as [`BashTool`].
pub struct SessionsSpawnTool {
    spawn_controller: Arc<SpawnController>,
    session_manager: Arc<SessionManager>,
}

impl SessionsSpawnTool {
    /// Create a new `SessionsSpawnTool` with the given dependencies.
    pub fn new(
        spawn_controller: Arc<SpawnController>,
        session_manager: Arc<SessionManager>,
    ) -> Self {
        Self {
            spawn_controller,
            session_manager,
        }
    }
}

#[async_trait]
impl Tool for SessionsSpawnTool {
    fn name(&self) -> &str {
        "sessions_spawn"
    }

    fn group(&self) -> &str {
        "sessions"
    }

    fn summary(&self) -> String {
        "Spawn a child session for a sub-agent".to_string()
    }

    fn detail(&self) -> String {
        "Create a child session that runs a sub-agent with a given task. \
         The child session inherits workspace context and runs independently. \
         Use `mode='run'` for one-shot tasks, `mode='session'` for persistent threads. \
         Returns the child session_id on success."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agentId": {
                    "type": "string",
                    "description": "Target agent ID to spawn"
                },
                "task": {
                    "type": "string",
                    "description": "Task description, injected as the child's first message"
                },
                "mode": {
                    "type": "string",
                    "enum": ["run", "session"],
                    "description": "Spawn mode: 'run' (one-shot) or 'session' (persistent)",
                    "default": "run"
                },
                "lightContext": {
                    "type": "boolean",
                    "description": "Use minimal bootstrap for the child session",
                    "default": false
                },
                "workspace": {
                    "type": "string",
                    "description": "Override workspace directory for the child session"
                },
                "label": {
                    "type": "string",
                    "description": "Short label for the child session"
                },
                "model": {
                    "type": "string",
                    "description": "Override the target agent's default model"
                },
                "fork": {
                    "type": "boolean",
                    "description": "是否 fork 父 agent 上下文：fork=true 时子 session 在 task 之前注入父 agent 的完整对话历史（不含 system prompt），使子 agent 继承父 agent 的上下文认知",
                    "default": false
                }
            },
            "required": ["task"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: true,
            ..Default::default()
        }
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        // 1. Extract parameters
        let task = args
            .get("task")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolCallError::InvalidArgs("missing required field 'task'".into()))?;
        let agent_id = args.get("agentId").and_then(|v| v.as_str());
        let light_context = args
            .get("lightContext")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let workspace = args.get("workspace").and_then(|v| v.as_str());
        let mode_str = args.get("mode").and_then(|v| v.as_str()).unwrap_or("run");
        let mode = match mode_str {
            "session" => SpawnMode::Session,
            _ => SpawnMode::Run,
        };
        let fork = args.get("fork").and_then(|v| v.as_bool()).unwrap_or(false);

        // 2. Get parent session_id from context
        let parent_session_id = ctx.session_id.as_deref().ok_or_else(|| {
            ToolCallError::ExecutionFailed("no session_id in tool context".into())
        })?;

        // 3. Validate spawn request
        let config = self
            .spawn_controller
            .validate(parent_session_id, agent_id)
            .await
            .map_err(|e| {
                ToolCallError::ExecutionFailed(format!("spawn validation failed: {}", e))
            })?;

        // 4. Get parent depth
        let parent_depth = self
            .session_manager
            .get_session_depth(parent_session_id)
            .await
            .unwrap_or(0);

        // 5. Create child session
        let child_session_id = self
            .session_manager
            .create_child_session(
                &config,
                parent_session_id,
                parent_depth + 1,
                task,
                light_context,
                workspace,
                mode,
                fork,
            )
            .await
            .map_err(|e| {
                ToolCallError::ExecutionFailed(format!("child session creation failed: {}", e))
            })?;

        // 6. Return result
        Ok(ToolResult {
            data: json!({
                "session_id": child_session_id,
                "agent_id": config.id,
                "depth": parent_depth + 1,
                "mode": mode_str,
            }),
            new_messages: vec![],
            context_modifier: None,
        })
    }
}
