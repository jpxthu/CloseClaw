//! Built-in ExecutePlan tool.
//!
//! Provides natural-language trigger for plan execution — the tool
//! equivalent of the `/execute` slash command. When the agent calls
//! this tool, the framework presents a user confirmation dialog
//! (confirm_pending). On confirmation, the session transitions to
//! Auto Mode and begins executing the plan steps.
//!
//! Supports two execution paths:
//! - **Same session**: the current session enters Auto Mode.
//! - **New session**: a new child session is created with the plan
//!   content injected as initial context, directly entering Auto Mode.

use crate::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};

use async_trait::async_trait;
use closeclaw_gateway::SessionManager;
use closeclaw_session::plan_file::{resolve_plan_by_name, PlanResolveError};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::builtin::plan_exec_confirm::{PlanExecConfirmFlow, PlanExecMetadata};

/// Natural-language execution trigger tool.
///
/// The agent calls this tool to start plan execution. The tool
/// returns a `confirm_pending` result, prompting the framework to
/// display a user confirmation dialog. On confirmation, the plan
/// enters Auto Mode for execution.
pub struct ExecutePlanTool {
    session_manager: Arc<SessionManager>,
    confirm_flow: Arc<PlanExecConfirmFlow>,
}

impl ExecutePlanTool {
    /// Creates a new `ExecutePlanTool`.
    pub fn new(
        session_manager: Arc<SessionManager>,
        confirm_flow: Arc<PlanExecConfirmFlow>,
    ) -> Self {
        Self {
            session_manager,
            confirm_flow,
        }
    }
}

#[async_trait]
impl Tool for ExecutePlanTool {
    fn name(&self) -> &str {
        "execute_plan"
    }

    fn group(&self) -> &str {
        "plan"
    }

    fn summary(&self) -> String {
        "Trigger plan execution with user confirmation".to_string()
    }

    fn detail(&self) -> String {
        "Trigger execution of a plan by name. This is the natural-language \
         equivalent of the `/execute` slash command. The tool returns a \
         confirm_pending result, prompting the user to confirm execution. \
         \n\nOn confirmation, the session transitions to Auto Mode \
         and begins executing the plan steps sequentially. \
         \n\nSupports two execution paths: \
         \n- Same session: the current session enters Auto Mode. \
         \n- New session: a new child session is created with the plan \
         content injected as initial context. \
         \n\nAn optional additional instruction can be provided to inject a user \
         message when the plan enters Auto Mode."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "plan_name": {
                    "type": "string",
                    "description": "Name of the plan to execute (resolved under \
                        workspace/plans/). Exact match takes priority, then prefix, \
                        then substring. If omitted, uses the plan from the current \
                        session's plan state."
                },
                "plan_file_path": {
                    "type": "string",
                    "description": "Full path to the plan file to execute (legacy). \
                        Prefer plan_name. If omitted, uses plan_name or the current \
                        session's plan state."
                },
                "additional_instruction": {
                    "type": "string",
                    "description": "Optional instruction injected as a user message \
                        when the plan enters Auto Mode. Empty or whitespace-only \
                        values are treated as absent."
                },
                "step_selection": {
                    "type": "array",
                    "items": {
                        "type": "integer"
                    },
                    "description": "Optional array of step indices to execute (0-based). \
                        If omitted, all steps are executed."
                },
                "new_session": {
                    "type": "boolean",
                    "description": "When true, create a new child session for execution \
                        instead of using the current session. The new session receives \
                        the plan content as initial context and enters Auto Mode directly.",
                    "default": false
                }
            },
            "required": []
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: true,
            is_read_only: false,
            is_destructive: false,
            is_expensive: false,
            is_deferred_by_default: false,
        }
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        let session_id = ctx.session_id.as_deref().ok_or_else(|| {
            ToolCallError::ExecutionFailed("no session_id in tool context".to_string())
        })?;

        let plan_name = Self::parse_plan_name(&args);
        let plan_file_path = Self::parse_plan_file_path(&args);
        let additional_instruction = Self::parse_additional_instruction(&args);

        // Load plan state only as fallback when no direct plan path is provided
        let plan_state = if plan_name.is_none() && plan_file_path.is_none() {
            Some(self.load_plan_state(session_id).await?)
        } else {
            None
        };
        let effective_path =
            self.resolve_effective_path(&plan_name, &plan_file_path, plan_state.as_ref(), ctx)?;
        // Refresh application-layer access timestamp so the plan
        // does not get archived prematurely after being loaded.
        {
            let path = std::path::Path::new(&effective_path);
            if path.exists() {
                if let Err(e) = closeclaw_session::plan_file::touch_access_timestamp(path) {
                    tracing::warn!(
                        plan_file = %effective_path,
                        error = %e,
                        "failed to touch access timestamp after plan load"
                    );
                }
            }
        }
        let step_selection = Self::parse_step_selection(&args);
        let new_session = Self::parse_new_session(&args);

        let meta = PlanExecMetadata {
            plan_file_path: effective_path.clone(),
            step_selection: step_selection.clone(),
            new_session,
            additional_instruction: additional_instruction.clone(),
        };
        let confirmation_id = self.confirm_flow.submit(session_id, meta).await;

        Ok(ToolResult {
            data: json!({
                "status": "confirm_pending",
                "confirmation_id": confirmation_id,
                "message": "Plan execution pending user confirmation",
                "plan_file_path": effective_path,
                "new_session": new_session,
            }),
            new_messages: Vec::new(),
            context_modifier: None,
        })
    }
}

// ── Private helpers ─────────────────────────────────────────────────────

impl ExecutePlanTool {
    /// Parse optional `plan_name` from tool arguments.
    fn parse_plan_name(args: &Value) -> Option<String> {
        args.get("plan_name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
    }

    /// Parse optional `plan_file_path` from tool arguments.
    fn parse_plan_file_path(args: &Value) -> Option<String> {
        args.get("plan_file_path")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
    }

    /// Parse optional `additional_instruction` from tool arguments.
    fn parse_additional_instruction(args: &Value) -> Option<String> {
        args.get("additional_instruction")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
    }

    /// Load the plan state for the current session.
    async fn load_plan_state(
        &self,
        session_id: &str,
    ) -> Result<closeclaw_common::PlanState, ToolCallError> {
        self.session_manager
            .get_plan_state(session_id)
            .await
            .ok_or_else(|| {
                ToolCallError::InvalidArgs(
                    "当前没有活跃的 plan。请先用 /plan <任务描述> 创建一个 plan。".to_string(),
                )
            })
    }

    /// Resolve the effective plan file path.
    ///
    /// Resolution order:
    /// 1. `plan_name` — resolved under workspace/plans/ by name
    /// 2. `plan_file_path` — direct path (legacy)
    /// 3. `plan_state.plan_file_path` — fallback from session
    fn resolve_effective_path(
        &self,
        plan_name: &Option<String>,
        plan_file_path: &Option<String>,
        plan_state: Option<&closeclaw_common::PlanState>,
        ctx: &ToolContext,
    ) -> Result<String, ToolCallError> {
        if let Some(name) = plan_name {
            let workdir_path = ctx.workdir.as_ref().map(|w| w.path.as_str()).unwrap_or(".");
            let workdir = std::path::Path::new(workdir_path);
            return resolve_plan_by_name(workdir, name)
                .map(|p| p.to_string_lossy().into_owned())
                .map_err(|e| match e {
                    PlanResolveError::NotFound { name: err_name } => {
                        let display_name = if err_name.is_empty() {
                            name.clone()
                        } else {
                            err_name
                        };
                        ToolCallError::InvalidArgs(format!("未找到名为 '{}' 的 plan", display_name))
                    }
                    PlanResolveError::Ambiguous { name, candidates } => ToolCallError::InvalidArgs(
                        format!("plan 名称 '{}' 歧义，候选：{}", name, candidates.join(", ")),
                    ),
                });
        }
        Self::resolve_plan_path(plan_file_path, plan_state)
    }

    /// Resolve the effective plan file path from a direct path or plan state.
    ///
    /// Uses the provided `plan_file_path` if given, otherwise falls back
    /// to the path stored in `plan_state`.
    fn resolve_plan_path(
        plan_file_path: &Option<String>,
        plan_state: Option<&closeclaw_common::PlanState>,
    ) -> Result<String, ToolCallError> {
        match plan_file_path {
            Some(p) => {
                if !std::path::Path::new(p).exists() {
                    return Err(ToolCallError::InvalidArgs(format!(
                        "plan 文件不存在：{}",
                        p
                    )));
                }
                Ok(p.clone())
            }
            None => match plan_state {
                Some(ps) => {
                    if ps.plan_file_path.is_empty() {
                        return Err(ToolCallError::InvalidArgs(
                            "当前 plan 没有关联的 plan 文件，无法执行。".to_string(),
                        ));
                    }
                    Ok(ps.plan_file_path.clone())
                }
                None => Err(ToolCallError::InvalidArgs(
                    "当前没有活跃的 plan。请先用 /plan <任务描述> 创建一个 plan，或通过 plan_name 指定 plan。"
                        .to_string(),
                )),
            },
        }
    }

    /// Parse the optional `step_selection` array from tool arguments.
    fn parse_step_selection(args: &Value) -> Option<Vec<usize>> {
        args.get("step_selection")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_u64)
                    .map(|i| i as usize)
                    .collect()
            })
            .filter(|v: &Vec<usize>| !v.is_empty())
    }

    /// Parse the optional `new_session` flag from tool arguments.
    fn parse_new_session(args: &Value) -> bool {
        args.get("new_session")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}
