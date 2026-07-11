//! Built-in sessions_spawn tool — creates child sessions for sub-agents.

use super::prompt_template::PromptTemplate;
use crate::permission_check::is_session_sub_agent;
use crate::{SpawnValidator, Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use closeclaw_gateway::session_manager::{SessionManager, SpawnMode};
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::engine_risk::RiskLevel;
use closeclaw_permission::engine::engine_types::{Caller, PermissionRequestBody};

use async_trait::async_trait;
use closeclaw_agent::AgentConfigLookup;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

/// Tool that spawns child sessions for sub-agent execution.
///
/// Holds trait-object references for spawn-time validation and config lookup,
/// following the same constructor-injection pattern as [`BashTool`].
pub struct SessionsSpawnTool {
    spawn_validator: Arc<dyn SpawnValidator>,
    session_manager: Arc<SessionManager>,
    agent_config_lookup: Arc<dyn AgentConfigLookup>,
    approval_flow: Arc<TokioMutex<ApprovalFlow>>,
}

/// Parsed arguments for a `sessions_spawn` tool call.
pub(crate) struct SpawnArgs {
    task: String,
    agent_id: Option<String>,
    light_context: bool,
    workspace: Option<String>,
    mode: SpawnMode,
    mode_str: String,
    fork: bool,
    allowed_tools: Option<Vec<String>>,
    prompt_template: Option<PromptTemplate>,
    pub(crate) model: Option<String>,
}

impl SessionsSpawnTool {
    /// Create a new `SessionsSpawnTool` with the given dependencies.
    pub fn new(
        spawn_validator: Arc<dyn SpawnValidator>,
        session_manager: Arc<SessionManager>,
        agent_config_lookup: Arc<dyn AgentConfigLookup>,
        approval_flow: Arc<TokioMutex<ApprovalFlow>>,
    ) -> Self {
        Self {
            spawn_validator,
            session_manager,
            agent_config_lookup,
            approval_flow,
        }
    }

    /// Parse the raw JSON arguments into typed [`SpawnArgs`].
    pub(crate) fn parse_args(args: &Value) -> Result<SpawnArgs, ToolCallError> {
        let task = args
            .get("task")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolCallError::InvalidArgs("missing required field 'task'".into()))?;
        let agent_id = args
            .get("agentId")
            .and_then(|v| v.as_str())
            .map(String::from);
        let light_context = args
            .get("lightContext")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let workspace = args
            .get("workspace")
            .and_then(|v| v.as_str())
            .map(String::from);
        let mode_str = args.get("mode").and_then(|v| v.as_str()).unwrap_or("run");
        let mode = match mode_str {
            "session" => SpawnMode::Session,
            _ => SpawnMode::Run,
        };
        let fork = args.get("fork").and_then(|v| v.as_bool()).unwrap_or(false);
        let allowed_tools = args
            .get("allowedTools")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<String>>()
            })
            .filter(|v| !v.is_empty());
        let prompt_template = args
            .get("promptTemplate")
            .and_then(|v| v.as_str())
            .map(|s| s.parse::<PromptTemplate>())
            .transpose()
            .map_err(|e| ToolCallError::InvalidArgs(e.to_string()))?;
        let model = args.get("model").and_then(|v| v.as_str()).map(String::from);
        Ok(SpawnArgs {
            task: task.to_string(),
            agent_id,
            light_context,
            workspace,
            mode,
            mode_str: mode_str.to_string(),
            fork,
            allowed_tools,
            prompt_template,
            model,
        })
    }

    /// Create a child session for the given config and parameters.
    ///
    /// Delegates to [`SessionManager::create_child_session`] with error mapping.
    #[allow(clippy::too_many_arguments)]
    async fn create_child(
        &self,
        config: &closeclaw_config::agents::ResolvedAgentConfig,
        parent_session_id: &str,
        parent_depth: u32,
        task: &str,
        light_context: bool,
        workspace: Option<&str>,
        mode: SpawnMode,
        fork: bool,
        allowed_tools: Option<Vec<String>>,
        model: Option<&str>,
        parent_subagents_model: Option<&str>,
        max_spawn_depth: u32,
    ) -> Result<String, ToolCallError> {
        self.session_manager
            .create_child_session(
                config,
                parent_session_id,
                parent_depth + 1,
                task,
                light_context,
                workspace,
                mode,
                fork,
                allowed_tools,
                model,
                parent_subagents_model,
                max_spawn_depth,
            )
            .await
            .map_err(|e| {
                ToolCallError::ExecutionFailed(format!("child session creation failed: {}", e))
            })
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
        let fork_desc = "是否 fork 父 agent 上下文：fork=true 时子 session ".to_owned()
            + "在 task 之前注入父 agent 的完整对话历史"
            + "（不含 system prompt），"
            + "使子 agent 继承父 agent 的上下文认知";
        let tools_desc = "Optional whitelist of tools the child session may ".to_owned()
            + "use. When provided, only these tools are available"
            + " to the child agent.";
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
                    "description": fork_desc,
                    "default": false
                },
                "allowedTools": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": tools_desc
                },
                "promptTemplate": {
                    "type": "string",
                    "enum": ["explore", "validation", "plan", "executor"],
                    "description": "Built-in prompt template to prepend to the task. 'explore' constrains read-only research; 'validation' enforces structured audit output; 'plan' constrains to read-only architect perspective; 'executor' runs with full toolset under review."
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
        let spawn_args = Self::parse_args(&args)?;

        // Plan Mode does not allow fork (context inheritance) — design doc:
        // "Plan Mode 不引入 Fork（上下文继承）机制"
        if ctx.session_mode == Some(closeclaw_common::SessionMode::Plan) && spawn_args.fork {
            return Err(ToolCallError::InvalidArgs(
                "fork is not allowed in Plan Mode. Use normal spawn for independent tasks.".into(),
            ));
        }

        let parent_session_id = ctx.session_id.as_deref().ok_or_else(|| {
            ToolCallError::ExecutionFailed("no session_id in tool context".into())
        })?;
        let spawn_result = match self
            .spawn_validator
            .validate_spawn(parent_session_id, spawn_args.agent_id.as_deref())
            .await
        {
            Ok(result) => result,
            Err(crate::SpawnError::PermissionDenied { agent_id, reason }) => {
                let caller = Caller {
                    user_id: String::new(),
                    agent: ctx.agent_id.clone(),
                    creator_id: String::new(),
                };
                let body = PermissionRequestBody::InterAgentMsg {
                    from: ctx.agent_id.clone(),
                    to: agent_id,
                };
                let session_id = ctx.session_id.as_deref().unwrap_or("");
                let mut flow = self.approval_flow.lock().await;
                let is_sub_agent = is_session_sub_agent(&self.session_manager, session_id).await;
                if let Some(request_id) =
                    flow.submit_denial(&caller, &body, RiskLevel::Medium, session_id, is_sub_agent)
                {
                    return Ok(ToolResult {
                        data: json!({
                            "status": "approval_pending",
                            "request_id": request_id,
                            "message": "Operation pending owner approval",
                        }),
                        new_messages: vec![],
                        context_modifier: None,
                    });
                }
                return Err(ToolCallError::PermissionDenied(reason));
            }
            Err(other) => {
                return Err(ToolCallError::ExecutionFailed(format!(
                    "spawn validation failed: {}",
                    other
                )));
            }
        };
        let config = spawn_result.config;
        let effective_max_spawn_depth = spawn_result.effective_max_spawn_depth;
        // Look up the parent agent's subagents.model config
        // (used as priority level 2 in the model priority chain).
        let parent_agent_id = self.session_manager.get_chat_id(parent_session_id).await;
        let parent_subagents_model: Option<String> = match &parent_agent_id {
            Some(id) => self
                .agent_config_lookup
                .lookup_agent_config(id)
                .await
                .and_then(|c| c.subagents_model)
                .map(|m| m.primary),
            None => None,
        };
        let parent_depth = self
            .session_manager
            .get_session_depth(parent_session_id)
            .await
            .unwrap_or(0);
        // Prepend prompt template prefix to task if specified
        let effective_task = match spawn_args.prompt_template {
            Some(tpl) => format!("{}\n\n{}", tpl.prefix(), spawn_args.task),
            None => spawn_args.task.clone(),
        };
        // Apply template default tool whitelist when no explicit allowedTools
        let effective_allowed_tools = match (&spawn_args.allowed_tools, &spawn_args.prompt_template)
        {
            (Some(tools), _) => Some(tools.clone()),
            (None, Some(tpl)) => tpl
                .default_allowed_tools()
                .map(|t| t.into_iter().map(String::from).collect()),
            (None, None) => None,
        };
        let child_session_id = self
            .create_child(
                &config,
                parent_session_id,
                parent_depth,
                &effective_task,
                spawn_args.light_context,
                spawn_args.workspace.as_deref(),
                spawn_args.mode,
                spawn_args.fork,
                effective_allowed_tools,
                spawn_args.model.as_deref(),
                parent_subagents_model.as_deref(),
                effective_max_spawn_depth,
            )
            .await?;
        Ok(ToolResult {
            data: json!({
                "session_id": child_session_id,
                "agent_id": config.id,
                "depth": parent_depth + 1,
                "mode": spawn_args.mode_str,
            }),
            new_messages: vec![],
            context_modifier: None,
        })
    }
}
