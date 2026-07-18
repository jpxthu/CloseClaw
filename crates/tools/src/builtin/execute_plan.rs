//! Built-in ExecutePlan tool.
//!
//! Provides natural-language trigger for plan execution — the tool
//! equivalent of the `/execute` slash command. When the agent calls
//! this tool, the framework presents a user confirmation dialog
//! (approval_pending). On approval, the session transitions from
//! Plan Mode to Auto Mode and begins executing the plan steps.
//!
//! Supports two execution paths:
//! - **Same session**: the current session enters Auto Mode.
//! - **New session**: a new child session is created with the plan
//!   content injected as initial context, directly entering Auto Mode.

use crate::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};

use async_trait::async_trait;
use closeclaw_common::{PlanStatus, SessionMode};
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_session::plan_file::parse_plan_status_from_file;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

/// Natural-language execution trigger tool.
///
/// The agent calls this tool to start plan execution. The tool
/// returns an `approval_pending` result, prompting the framework to
/// display a user confirmation dialog. On approval, the plan enters
/// Auto Mode for execution.
pub struct ExecutePlanTool {
    session_manager: Arc<SessionManager>,
    approval_flow: Arc<TokioMutex<ApprovalFlow>>,
}

impl ExecutePlanTool {
    /// Creates a new `ExecutePlanTool`.
    pub fn new(
        session_manager: Arc<SessionManager>,
        approval_flow: Arc<TokioMutex<ApprovalFlow>>,
    ) -> Self {
        Self {
            session_manager,
            approval_flow,
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
        "Trigger execution of the current plan. This is the natural-language \
         equivalent of the `/execute` slash command. The tool returns an \
         approval_pending result, prompting the user to confirm execution. \
         \n\nOn approval, the session transitions from Plan Mode to Auto Mode \
         and begins executing the plan steps sequentially. \
         \n\nSupports two execution paths: \
         \n- Same session: the current session enters Auto Mode. \
         \n- New session: a new child session is created with the plan \
         content injected as initial context."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "plan_file_path": {
                    "type": "string",
                    "description": "Path to the plan file to execute. \
                        If omitted, uses the plan file from the current session's plan state."
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

        self.validate_plan_mode(session_id).await?;

        let plan_file_path = Self::parse_plan_file_path(&args);
        let plan_state = self.load_plan_state(session_id).await?;
        let effective_path = Self::resolve_plan_path(&plan_file_path, &plan_state)?;
        self.validate_plan_status(&effective_path, &plan_state)
            .await?;

        let step_selection = Self::parse_step_selection(&args);
        let new_session = Self::parse_new_session(&args);

        let request_id = uuid::Uuid::new_v4().to_string();
        self.store_plan_exec_metadata(&request_id, &effective_path, &step_selection, new_session)
            .await;

        Ok(ToolResult {
            data: json!({
                "status": "approval_pending",
                "request_id": request_id,
                "message": "Plan execution pending owner approval",
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
    /// Validate that the session is in Plan Mode.
    async fn validate_plan_mode(&self, session_id: &str) -> Result<(), ToolCallError> {
        let conv = self
            .session_manager
            .get_conversation_session(session_id)
            .await
            .ok_or_else(|| ToolCallError::ExecutionFailed("当前会话未激活".to_string()))?;
        let cs = conv.read().await;
        if cs.session_mode() != SessionMode::Plan {
            return Err(ToolCallError::InvalidArgs(
                "execute_plan 需要在 Plan Mode 下使用。先用 /plan <任务描述> 进入 Plan Mode。"
                    .to_string(),
            ));
        }
        Ok(())
    }

    /// Parse optional `plan_file_path` from tool arguments.
    fn parse_plan_file_path(args: &Value) -> Option<String> {
        args.get("plan_file_path")
            .and_then(Value::as_str)
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
    /// Uses the provided `plan_file_path` if given, otherwise falls back
    /// to the path stored in `plan_state`.
    fn resolve_plan_path(
        plan_file_path: &Option<String>,
        plan_state: &closeclaw_common::PlanState,
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
            None => {
                if plan_state.plan_file_path.is_empty() {
                    return Err(ToolCallError::InvalidArgs(
                        "当前 plan 没有关联的 plan 文件，无法执行。".to_string(),
                    ));
                }
                Ok(plan_state.plan_file_path.clone())
            }
        }
    }

    /// Read the plan file and validate that its status is Confirmed or Paused.
    async fn validate_plan_status(
        &self,
        effective_path: &str,
        plan_state: &closeclaw_common::PlanState,
    ) -> Result<(), ToolCallError> {
        let content = tokio::fs::read_to_string(effective_path)
            .await
            .map_err(|e| ToolCallError::ExecutionFailed(format!("无法读取 plan 文件：{}", e)))?;

        let file_status = parse_plan_status_from_file(&content).ok_or_else(|| {
            ToolCallError::ExecutionFailed("Plan 文件中未找到有效的状态字段。".to_string())
        })?;

        let effective_status = if plan_state.status != PlanStatus::Draft {
            plan_state.status
        } else {
            file_status
        };

        match effective_status {
            PlanStatus::Confirmed | PlanStatus::Paused => Ok(()),
            _ => Err(ToolCallError::InvalidArgs(
                "当前 plan 未就绪。请先使用 plan_approval 工具提交审批，或先暂停再恢复执行。"
                    .to_string(),
            )),
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

    /// Store plan execution metadata in the approval flow.
    async fn store_plan_exec_metadata(
        &self,
        request_id: &str,
        effective_path: &str,
        step_selection: &Option<Vec<usize>>,
        new_session: bool,
    ) {
        let mut flow = self.approval_flow.lock().await;
        flow.set_plan_exec_metadata(
            request_id,
            effective_path.to_string(),
            step_selection.clone(),
            new_session,
        );
    }
}
