//! Built-in sessions_steer tool — injects a new task into a persistent child session.

use crate::gateway::SessionManager;
use crate::tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Tool that steers a persistent child session by injecting a new task
/// into its pending message queue.
///
/// Only works on `mode=session` children owned by the calling parent.
pub struct SessionsSteerTool {
    session_manager: Arc<SessionManager>,
}

impl SessionsSteerTool {
    /// Create a new `SessionsSteerTool` with the given dependencies.
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
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
                "childId": {
                    "type": "string",
                    "description": "The session ID of the child session to steer"
                },
                "task": {
                    "type": "string",
                    "description": "The new task to inject into the child session"
                }
            },
            "required": ["childId", "task"]
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
            .get("childId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolCallError::InvalidArgs("missing required field 'childId'".into()))?;
        let task = args
            .get("task")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolCallError::InvalidArgs("missing required field 'task'".into()))?;

        // 2. Get parent session_id from context
        let parent_session_id = ctx.session_id.as_deref().ok_or_else(|| {
            ToolCallError::ExecutionFailed("no session_id in tool context".into())
        })?;

        // 3. Validate ownership
        self.session_manager
            .validate_child_ownership(parent_session_id, child_id)
            .await
            .ok_or_else(|| {
                ToolCallError::ExecutionFailed(
                    "child session not found or not owned by parent".into(),
                )
            })?;

        // 4. Steer the child session
        self.session_manager
            .steer_child(child_id, task)
            .await
            .map_err(ToolCallError::ExecutionFailed)?;

        // 5. Return result
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
