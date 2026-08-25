//! sessions_spawn tool — creates child sessions for sub-agents.

use super::prompt_template::PromptTemplate;
use super::SessionManagerOps;
use crate::spawn_validation::SpawnValidator;
use closeclaw_common::tool_trait::{
    PromptGenerationContext, Tool, ToolCallError, ToolContext, ToolFlags, ToolResult,
};

use async_trait::async_trait;
use closeclaw_agent::AgentConfigLookup;
use serde_json::{json, Value};
use std::sync::Arc;

/// Tool that spawns child sessions for sub-agent execution.
pub struct SessionsSpawnTool {
    spawn_validator: Arc<dyn SpawnValidator>,
    session_manager: Arc<dyn SessionManagerOps>,
    agent_config_lookup: Arc<dyn AgentConfigLookup>,
}

/// Parsed arguments for a `sessions_spawn` tool call.
pub(crate) struct SpawnArgs {
    task: String,
    agent_id: Option<String>,
    light_context: bool,
    workspace: Option<String>,
    mode: crate::spawn::SpawnMode,
    mode_str: String,
    fork: bool,
    pub(crate) allowed_tools: Option<Vec<String>>,
    pub(crate) prompt_template: Option<PromptTemplate>,
    pub(crate) model: Option<String>,
    pub(crate) timeout: Option<u64>,
    pub(crate) label: Option<String>,
    pub(crate) timeout_warning: Option<u64>,
    pub(crate) timeout_notify_interval_ratio: Option<f64>,
}

impl SessionsSpawnTool {
    /// Create a new `SessionsSpawnTool` with the given dependencies.
    pub fn new(
        spawn_validator: Arc<dyn SpawnValidator>,
        session_manager: Arc<dyn SessionManagerOps>,
        agent_config_lookup: Arc<dyn AgentConfigLookup>,
    ) -> Self {
        Self {
            spawn_validator,
            session_manager,
            agent_config_lookup,
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
            "session" => crate::spawn::SpawnMode::Session,
            _ => crate::spawn::SpawnMode::Run,
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
        let timeout = args.get("timeout").and_then(|v| v.as_u64());
        let label = args.get("label").and_then(|v| v.as_str()).map(String::from);
        let timeout_warning = args.get("timeoutWarning").and_then(|v| v.as_u64());
        let timeout_notify_interval_ratio = args
            .get("timeoutNotifyIntervalRatio")
            .and_then(|v| v.as_f64());
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
            timeout,
            label,
            timeout_warning,
            timeout_notify_interval_ratio,
        })
    }

    /// Create a child session for the given config and parameters.
    #[allow(clippy::too_many_arguments)]
    async fn create_child(
        &self,
        config: &closeclaw_config::agents::ResolvedAgentConfig,
        parent_session_id: &str,
        parent_depth: u32,
        task: &str,
        light_context: bool,
        workspace: Option<&str>,
        mode: crate::spawn::SpawnMode,
        fork: bool,
        allowed_tools: Option<Vec<String>>,
        model: Option<&str>,
        parent_subagents_model: Option<&str>,
        max_spawn_depth: u32,
        spawn_timeout: Option<u64>,
        label: Option<&str>,
        prompt_template_prefix: Option<&str>,
        timeout_warning_secs: Option<u64>,
        timeout_notify_interval_ratio: Option<f64>,
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
                spawn_timeout,
                label,
                prompt_template_prefix,
                timeout_warning_secs,
                timeout_notify_interval_ratio,
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

    fn generate_prompt(&self, context: &PromptGenerationContext) -> String {
        let mut parts = Vec::new();

        parts.push(spawn_prompt_when_to_use());
        spawn_prompt_add_budget_status(context, &mut parts);
        parts.push(spawn_prompt_usage_principles());
        parts.push(spawn_prompt_task_authoring_guidance());
        spawn_prompt_add_combination_suggestions(context, &mut parts);
        spawn_prompt_add_workdir_guidance(context, &mut parts);

        parts.join("\n\n")
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
                "timeout": {
                    "type": "integer",
                    "description": "Override the target agent's spawn timeout (seconds). Takes highest priority in the timeout resolution chain."
                },
                "timeoutWarning": {
                    "type": "integer",
                    "description": "Override the target agent's timeout warning (seconds). When the sub-agent has been running for this many seconds, cyclic warning notifications begin. Takes highest priority in the timeout_warning resolution chain (spawn args -> target agent config -> global default)."
                },
                "timeoutNotifyIntervalRatio": {
                    "type": "number",
                    "description": "Override the interval ratio for cyclic warning notifications (relative to timeoutWarning). Must be >=0.1 and <=2.0, default 0.5."
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
                    "description": "Built-in prompt template to prepend to the task. 'explore' constrains read-only research; 'validation' enforces structured audit output; 'plan' constrains to read-only architect perspective; 'executor' autonomously executes plan tasks."
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

        // ── Step 1: Precondition checks (depth, concurrency, agentId, whitelist) ──
        let spawn_result = match self
            .spawn_validator
            .validate_spawn(parent_session_id, spawn_args.agent_id.as_deref())
            .await
        {
            Ok(result) => result,
            Err(crate::spawn_validation::SpawnError::PermissionDenied { .. }) => {
                // validate_spawn should not return PermissionDenied after
                // two-step separation (permission check is step 2).
                // If this ever fires, it indicates a bug in the two-step
                // separation — log it as an error for diagnostics.
                tracing::error!(
                    parent_session_id = %parent_session_id,
                    "BUG: validate_spawn unexpectedly returned PermissionDenied\n                    after two-step separation. This should never happen —\n                    permission check is a separate step in check_spawn_permission."
                );
                return Err(ToolCallError::ExecutionFailed(
                    "internal error: unexpected PermissionDenied from validate_spawn".into(),
                ));
            }
            Err(other) => {
                return Err(ToolCallError::ExecutionFailed(format!(
                    "spawn validation failed: {}",
                    other
                )));
            }
        };

        // ── Step 2: Permission check (tools layer triggers) ──
        match self
            .spawn_validator
            .check_spawn_permission(parent_session_id, &spawn_result)
            .await
        {
            Ok(()) => {}
            Err(crate::spawn_validation::SpawnError::PermissionDenied { reason, .. }) => {
                return Err(ToolCallError::PermissionDenied(reason));
            }
            Err(other) => {
                return Err(ToolCallError::ExecutionFailed(format!(
                    "spawn permission check failed: {}",
                    other
                )));
            }
        }

        let config = spawn_result.config;
        let effective_max_spawn_depth = spawn_result.effective_max_spawn_depth;
        let mut spawn_timeout = spawn_result.spawn_timeout;
        if let Some(arg_timeout) = spawn_args.timeout {
            spawn_timeout = Some(arg_timeout);
        }
        // Apply timeout_warning priority chain: spawn args > target agent config > global default.
        let mut timeout_warning_secs = spawn_result.timeout_warning_secs;
        let mut timeout_notify_interval_ratio = spawn_result.timeout_notify_interval_ratio;
        if spawn_args.timeout_warning.is_some() {
            timeout_warning_secs = spawn_args.timeout_warning;
        }
        if spawn_args.timeout_notify_interval_ratio.is_some() {
            timeout_notify_interval_ratio = spawn_args.timeout_notify_interval_ratio;
        }
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
        let prompt_template_prefix = spawn_args.prompt_template.as_ref().map(|tpl| tpl.prefix());

        let child_session_id = self
            .create_child(
                &config,
                parent_session_id,
                parent_depth,
                &spawn_args.task,
                spawn_args.light_context,
                spawn_args.workspace.as_deref(),
                spawn_args.mode,
                spawn_args.fork,
                spawn_args.allowed_tools.clone(),
                spawn_args.model.as_deref(),
                parent_subagents_model.as_deref(),
                effective_max_spawn_depth,
                spawn_timeout,
                spawn_args.label.as_deref(),
                prompt_template_prefix,
                timeout_warning_secs,
                timeout_notify_interval_ratio,
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

// --- Prompt generation helpers ---

/// "When to use" section for the sessions_spawn tool prompt.
fn spawn_prompt_when_to_use() -> String {
    "Use sessions_spawn to create child sessions for sub-agent execution. \
     Ideal when you need: parallel task execution across multiple agents, \
     isolated execution environments (sandboxed context), long-running tasks \
     that should not block the parent session, or multi-agent coordination \
     where different agents handle different responsibilities."
        .to_string()
}

/// Append budget-awareness text to the prompt parts.
/// Since budget filtering is now handled at session creation time
/// (whitelist removal), this always reports the tool as available.
fn spawn_prompt_add_budget_status(_context: &PromptGenerationContext, parts: &mut Vec<String>) {
    parts.push(
        "sessions_spawn is available. Spawn budget is managed at session \
         creation time — use with reasonable caution."
            .to_string(),
    );
}

/// "Usage principles" section for the sessions_spawn tool prompt.
fn spawn_prompt_usage_principles() -> String {
    "Set reasonable timeouts for spawned sessions — avoid indefinite \
     execution. Prefer mode='run' for one-shot tasks and mode='session' \
     only when persistent interaction is needed. Avoid excessive spawning: \
     consolidate related tasks into fewer child sessions when possible."
        .to_string()
}

/// Task authoring guidance for the sessions_spawn tool prompt.
///
/// Design doc reference: docs/design/agent/agent-spawn.md
/// §子 Agent 提示词工程 > 父 Agent 的 Task 编写指引.
fn spawn_prompt_task_authoring_guidance() -> String {
    "Task authoring guidelines:\n\n\
     - Brief the child like a smart colleague who just walked into \
       the room — say what to do and why.\n\
     - Don't delegate synthesis or judgment: you understand and \
       decide, the child executes.\n\
     - Use fork=true when the child needs full conversational \
       context; use plain spawn for independent subtasks."
        .to_string()
}

/// Append combination suggestions based on available tools.
fn spawn_prompt_add_combination_suggestions(
    context: &PromptGenerationContext,
    parts: &mut Vec<String>,
) {
    let mut suggestions = Vec::new();

    let has_yield = context
        .available_tool_names
        .iter()
        .any(|t| t == "sessions_yield");
    let has_steer = context
        .available_tool_names
        .iter()
        .any(|t| t == "sessions_steer");
    let has_kill = context
        .available_tool_names
        .iter()
        .any(|t| t == "sessions_kill");

    if has_yield {
        suggestions.push("sessions_yield (wait for child completion before continuing)");
    }
    if has_steer {
        suggestions.push("sessions_steer (redirect a persistent child session's task)");
    }
    if has_kill {
        suggestions.push("sessions_kill (force-terminate a child session if needed)");
    }

    if !suggestions.is_empty() {
        parts.push(format!("Lifecycle management: {}.", suggestions.join(", ")));
    }
}

/// Append workdir-based path guidance to the prompt parts.
fn spawn_prompt_add_workdir_guidance(context: &PromptGenerationContext, parts: &mut Vec<String>) {
    if let Some(ref wd) = context.workdir {
        parts.push(format!(
            "Current workspace: {}. \
             Child sessions inherit this workspace by default unless \
             a different workspace is specified via the workspace parameter.",
            wd.path
        ));
    }
}
