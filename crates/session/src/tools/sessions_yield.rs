//! sessions_yield tool — signals the session to enter Waiting state.

use super::SessionManagerOps;
use closeclaw_common::tool_trait::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Tool that signals the current session to enter active Waiting state.
pub struct SessionsYieldTool {
    session_manager: Arc<dyn SessionManagerOps>,
}

impl SessionsYieldTool {
    /// Create a new `SessionsYieldTool`.
    pub fn new(session_manager: Arc<dyn SessionManagerOps>) -> Self {
        Self { session_manager }
    }
}

#[async_trait]
impl Tool for SessionsYieldTool {
    fn name(&self) -> &str {
        "sessions_yield"
    }

    fn group(&self) -> &str {
        "sessions"
    }

    fn summary(&self) -> String {
        "Yield current turn and wait for child sessions".to_string()
    }

    fn detail(&self) -> String {
        "End the current turn and enter Waiting state. \
         User messages are queued until all active child sessions complete. \
         The session resumes automatically after all children finish."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: true,
            is_read_only: false,
            ..Default::default()
        }
    }

    async fn call(&self, _args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        let session = ctx
            .session
            .as_deref()
            .ok_or_else(|| ToolCallError::ExecutionFailed("no session in tool context".into()))?;

        let session_id = ctx.session_id.as_deref().ok_or_else(|| {
            ToolCallError::ExecutionFailed("no session_id in tool context".into())
        })?;

        session.enter_waiting();

        self.session_manager
            .clone()
            .start_yield_timeout(session_id, &ctx.agent_id, None)
            .await;

        tracing::info!(
            session_id = %session_id,
            "sessions_yield: session entered Waiting state, turn will end"
        );

        Ok(ToolResult {
            data: json!({
                "status": "yielded",
                "message": "Session entered Waiting state. Turn ended."
            }),
            new_messages: vec![],
            context_modifier: None,
        })
    }
}
