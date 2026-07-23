//! sessions_kill tool — terminates a persistent child session and releases resources.

use super::{build_approval_pending, SessionManagerOps};
use closeclaw_common::permission_types::CallerInfo;
use closeclaw_common::tool_trait::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Tool that kills a child session by stopping it (cascade)
/// and removing it from all tracking tables.
pub struct SessionsKillTool {
    session_manager: Arc<dyn SessionManagerOps>,
    permission_engine: closeclaw_common::permission_types::SharedPermissionEvaluator,
    approval_flow: closeclaw_common::permission_types::SharedApprovalSubmission,
}

impl SessionsKillTool {
    /// Create a new `SessionsKillTool` with the given dependencies.
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
impl Tool for SessionsKillTool {
    fn name(&self) -> &str {
        "sessions_kill"
    }

    fn group(&self) -> &str {
        "sessions"
    }

    fn summary(&self) -> String {
        "Terminate a persistent child session and release resources".to_string()
    }

    fn detail(&self) -> String {
        "Kill a child session by triggering a cascading \
         stop (cancels in-flight LLM requests and tool processes), then removing \
         it from all tracking tables. The archive is preserved. \
         Supports any mode (run / session). \
         Requires the child to be owned by the calling parent session."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sessionId": {
                    "type": "string",
                    "description": "The session ID of the child session to kill"
                }
            },
            "required": ["sessionId"]
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
            .kill_child(parent_session_id, child_id)
            .await
            .map_err(ToolCallError::ExecutionFailed)?;

        Ok(ToolResult {
            data: json!({
                "child_id": child_id,
                "status": "killed",
            }),
            new_messages: vec![],
            context_modifier: None,
        })
    }
}
