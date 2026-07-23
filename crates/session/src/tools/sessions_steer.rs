//! sessions_steer tool — injects a new task into a persistent child session.

use super::SessionManagerOps;
use closeclaw_common::permission_types::CallerInfo;
use closeclaw_common::tool_trait::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Build the standard `approval_pending` response payload.
fn build_approval_pending(request_id: String) -> Value {
    let mut m = serde_json::Map::new();
    m.insert("status".into(), "approval_pending".into());
    m.insert("request_id".into(), request_id.into());
    m.insert("message".into(), "Operation pending owner approval".into());
    Value::Object(m)
}

/// Tool that steers a persistent child session by injecting a new task
/// into its pending message queue.
pub struct SessionsSteerTool {
    session_manager: Arc<dyn SessionManagerOps>,
    permission_engine: closeclaw_common::permission_types::SharedPermissionEvaluator,
    approval_flow: closeclaw_common::permission_types::SharedApprovalSubmission,
}

impl SessionsSteerTool {
    /// Create a new `SessionsSteerTool` with the given dependencies.
    pub fn new(
        session_manager: Arc<dyn SessionManagerOps>,
        permission_engine: closeclaw_common::permission_types::SharedPermissionEvaluator,
        approval_flow: closeclaw_common::permission_types::SharedApprovalSubmission,
    ) -> Self {
        Self {
            session_manager,
            permission_engine,
            approval_flow,
        }
    }
}

#[async_trait]
impl Tool for SessionsSteerTool {
    fn name(&self) -> &str {
        "sessions_steer"
    }

    fn group(&self) -> &str {
        "sessions"
    }

    fn summary(&self) -> String {
        "Inject a new task into a persistent child session".to_string()
    }

    fn detail(&self) -> String {
        "Steer a persistent (mode=session) child session by injecting a new task \
         into its pending message queue. The task is enqueued (FIFO) and will be \
         consumed after the child's current turn completes. \
         Requires the child to be owned by the calling parent session."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sessionId": {
                    "type": "string",
                    "description": "The session ID of the child session to steer"
                },
                "task": {
                    "type": "string",
                    "description": "The new task to inject into the child session"
                }
            },
            "required": ["sessionId", "task"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: true,
            ..Default::default()
        }
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        let child_id = args
            .get("sessionId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolCallError::InvalidArgs("missing required field 'sessionId'".into())
            })?;
        let task = args
            .get("task")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolCallError::InvalidArgs("missing required field 'task'".into()))?;

        let parent_session_id = ctx.session_id.as_deref().ok_or_else(|| {
            ToolCallError::ExecutionFailed("no session_id in tool context".into())
        })?;

        let info = self
            .session_manager
            .validate_child_ownership(parent_session_id, child_id)
            .await
            .ok_or_else(|| {
                ToolCallError::ExecutionFailed(
                    "child session not found or not owned by parent".into(),
                )
            })?;

        if info.mode != crate::spawn::SpawnMode::Session {
            return Err(ToolCallError::ExecutionFailed(
                "steer is only allowed on mode=session children".into(),
            ));
        }

        // Cross-Agent permission check
        let response = self
            .permission_engine
            .evaluate_inter_agent(&ctx.agent_id, &info.agent_id)
            .await;
        if let closeclaw_common::permission_types::PermissionEvalResponse::Denied {
            reason,
            risk_level,
        } = response
        {
            let caller = CallerInfo {
                user_id: String::new(),
                agent: ctx.agent_id.clone(),
                creator_id: String::new(),
            };
            let session_id = ctx.session_id.as_deref().unwrap_or("");
            let flow = self.approval_flow.lock().await;
            if let Some(request_id) = flow.submit_inter_agent_denial(
                &caller,
                &ctx.agent_id,
                &info.agent_id,
                risk_level,
                session_id,
                false,
            ) {
                let data = build_approval_pending(request_id);
                return Ok(ToolResult {
                    data,
                    new_messages: vec![],
                    context_modifier: None,
                });
            }
            return Err(ToolCallError::ExecutionFailed(format!(
                "inter-agent communication denied: {}",
                reason
            )));
        }

        self.session_manager
            .steer_child(child_id, task)
            .await
            .map_err(ToolCallError::ExecutionFailed)?;

        Ok(ToolResult {
            data: json!({
                "child_id": child_id,
                "task": task,
            }),
            new_messages: vec![],
            context_modifier: None,
        })
    }
}
