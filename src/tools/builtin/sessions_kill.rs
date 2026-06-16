//! Built-in sessions_kill tool — terminates a persistent child session and releases resources.

use crate::gateway::SessionManager;
use crate::permission::engine::engine_eval::PermissionEngine;
use crate::permission::engine::engine_types::{Caller, PermissionRequest, PermissionRequestBody};
use crate::tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Tool that kills a persistent child session by stopping it (cascade)
/// and removing it from all tracking tables.
///
/// Only works on `mode=session` children owned by the calling parent.
pub struct SessionsKillTool {
    session_manager: Arc<SessionManager>,
    permission_engine: Arc<PermissionEngine>,
}

impl SessionsKillTool {
    /// Create a new `SessionsKillTool` with the given dependencies.
    pub fn new(
        session_manager: Arc<SessionManager>,
        permission_engine: Arc<PermissionEngine>,
    ) -> Self {
        Self {
            session_manager,
            permission_engine,
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
        "Kill a persistent (mode=session) child session by triggering a cascading \
         stop (cancels in-flight LLM requests and tool processes), then removing \
         it from all tracking tables. The archive is preserved. \
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
        // 1. Extract parameters
        let child_id = args
            .get("sessionId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolCallError::InvalidArgs("missing required field 'sessionId'".into())
            })?;

        // 2. Get parent session_id from context
        let parent_session_id = ctx.session_id.as_deref().ok_or_else(|| {
            ToolCallError::ExecutionFailed("no session_id in tool context".into())
        })?;

        // 3. Validate ownership
        let child_info = self
            .session_manager
            .validate_child_ownership(parent_session_id, child_id)
            .await
            .ok_or_else(|| {
                ToolCallError::ExecutionFailed(
                    "child session not found or not owned by parent".into(),
                )
            })?;

        // 4. Permission engine check — cross-agent communication
        let user_id = self
            .session_manager
            .get_sender_id(parent_session_id)
            .await
            .unwrap_or_default();
        let caller = Caller {
            user_id,
            agent: ctx.agent_id.clone(),
            ..Caller::default()
        };
        let body = PermissionRequestBody::InterAgentMsg {
            from: ctx.agent_id.clone(),
            to: child_info.agent_id.clone(),
        };
        match self.permission_engine.evaluate(
            PermissionRequest::WithCaller {
                caller,
                request: body,
            },
            None,
        ) {
            crate::permission::engine::engine_types::PermissionResponse::Allowed { .. } => {}
            crate::permission::engine::engine_types::PermissionResponse::Denied {
                reason, ..
            } => {
                return Err(ToolCallError::ExecutionFailed(format!(
                    "permission denied: {}",
                    reason
                )));
            }
        }

        // 5. Kill the child session
        self.session_manager
            .kill_child(parent_session_id, child_id)
            .await
            .map_err(ToolCallError::ExecutionFailed)?;

        // 6. Return result
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
