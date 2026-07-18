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
use closeclaw_agent::AgentConfigLookup;
use closeclaw_common::{PlanStatus, SessionMode};
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::ApprovalFlow;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

/// Parse `PlanStatus` from a plan file's content.
///
/// Scans the file for the "| 状态 | <status> |" line and converts it
/// to the corresponding PlanStatus variant.
fn parse_plan_status_from_file(content: &str) -> Option<PlanStatus> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("| 状态 | ") {
            let status_str = rest.strip_suffix(" |")?.trim();
            return match status_str {
                "draft" => Some(PlanStatus::Draft),
                "confirmed" => Some(PlanStatus::Confirmed),
                "executing" => Some(PlanStatus::Executing),
                "paused" => Some(PlanStatus::Paused),
                "completed" => Some(PlanStatus::Completed),
                _ => None,
            };
        }
    }
    None
}

/// Natural-language execution trigger tool.
///
/// The agent calls this tool to start plan execution. The tool
/// returns an `approval_pending` result, prompting the framework to
/// display a user confirmation dialog. On approval, the plan enters
/// Auto Mode for execution.
pub struct ExecutePlanTool {
    session_manager: Arc<SessionManager>,
    #[allow(dead_code)]
    agent_config_lookup: Arc<dyn AgentConfigLookup>,
    approval_flow: Arc<TokioMutex<ApprovalFlow>>,
}

impl ExecutePlanTool {
    /// Creates a new `ExecutePlanTool`.
    pub fn new(
        session_manager: Arc<SessionManager>,
        agent_config_lookup: Arc<dyn AgentConfigLookup>,
        approval_flow: Arc<TokioMutex<ApprovalFlow>>,
    ) -> Self {
        Self {
            session_manager,
            agent_config_lookup,
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

        // ── Validate session is in Plan Mode ──────────────────────────
        {
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
        }

        // ── Parse optional plan_file_path ─────────────────────────────
        let plan_file_path = args
            .get("plan_file_path")
            .and_then(Value::as_str)
            .map(|s| s.to_string());

        // ── Load and validate plan state ──────────────────────────────
        let plan_state = match self.session_manager.get_plan_state(session_id).await {
            Some(ps) => ps,
            None => {
                return Err(ToolCallError::InvalidArgs(
                    "当前没有活跃的 plan。请先用 /plan <任务描述> 创建一个 plan。".to_string(),
                ));
            }
        };

        // Resolve plan file path: use provided path or fall back to plan state.
        let effective_path = match &plan_file_path {
            Some(p) => {
                if !std::path::Path::new(p).exists() {
                    return Err(ToolCallError::InvalidArgs(format!(
                        "plan 文件不存在：{}",
                        p
                    )));
                }
                p.clone()
            }
            None => {
                if plan_state.plan_file_path.is_empty() {
                    return Err(ToolCallError::InvalidArgs(
                        "当前 plan 没有关联的 plan 文件，无法执行。".to_string(),
                    ));
                }
                plan_state.plan_file_path.clone()
            }
        };

        // ── Read plan file and validate status ────────────────────────
        let content = std::fs::read_to_string(&effective_path)
            .map_err(|e| ToolCallError::ExecutionFailed(format!("无法读取 plan 文件：{}", e)))?;

        let file_status = parse_plan_status_from_file(&content).ok_or_else(|| {
            ToolCallError::ExecutionFailed("Plan 文件中未找到有效的状态字段。".to_string())
        })?;

        // Use in-memory status if non-default; otherwise trust file-parsed status.
        let effective_status = if plan_state.status != PlanStatus::Draft {
            plan_state.status
        } else {
            file_status
        };

        match effective_status {
            PlanStatus::Confirmed | PlanStatus::Paused => {
                // Valid status — proceed to approval
            }
            _ => {
                return Err(ToolCallError::InvalidArgs(
                    "当前 plan 未就绪。请先使用 plan_approval 工具提交审批，或先暂停再恢复执行。"
                        .to_string(),
                ));
            }
        }

        // ── Parse optional step_selection ──────────────────────────────
        let step_selection: Option<Vec<usize>> = args
            .get("step_selection")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_u64)
                    .map(|i| i as usize)
                    .collect()
            })
            .filter(|v: &Vec<usize>| !v.is_empty());

        // ── Parse new_session flag ─────────────────────────────────────
        let new_session = args
            .get("new_session")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // ── Build approval pending result ──────────────────────────────
        let request_id = uuid::Uuid::new_v4().to_string();

        // Store metadata for the approval flow to consume on approval.
        {
            let mut flow = self.approval_flow.lock().await;
            flow.set_plan_exec_metadata(
                &request_id,
                effective_path.clone(),
                step_selection.clone(),
                new_session,
            );
        }

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
