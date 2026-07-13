//! Built-in sessions_yield tool — signals the session to enter Waiting state.

use crate::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use async_trait::async_trait;
use closeclaw_gateway::session_manager::SessionManager;
use serde_json::{json, Value};
use std::sync::Arc;

/// Tool that signals the current session to enter active Waiting state.
///
/// When called, the session enters Waiting (yielding) and the current
/// turn ends immediately — no further LLM requests are made until
/// all active child sessions complete and the session resumes.
///
/// A configurable timeout timer is started. If child sessions do not
/// complete within the timeout, they are terminated and the session
/// resumes with a timeout notification.
///
/// This tool is registered in the `sessions` group alongside
/// `sessions_spawn`, `sessions_steer`, and `sessions_kill`.
pub struct SessionsYieldTool {
    session_manager: Arc<SessionManager>,
}

impl SessionsYieldTool {
    /// Create a new `SessionsYieldTool`.
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
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
        // 1. Access the session via the ToolContext's session reference.
        let session = ctx
            .session
            .as_deref()
            .ok_or_else(|| ToolCallError::ExecutionFailed("no session in tool context".into()))?;

        let session_id = ctx.session_id.as_deref().ok_or_else(|| {
            ToolCallError::ExecutionFailed("no session_id in tool context".into())
        })?;

        // 2. Call enter_waiting on the session to set the yielding flag.
        //    The Gateway checks this flag after the LLM call completes
        //    and skips draining pending messages (ending the turn).
        session.enter_waiting();

        // 3. Start the yield timeout timer via SessionManager.
        //    If child sessions don't complete within the timeout,
        //    they are terminated and the session resumes.
        self.session_manager
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sessions_yield_tool_metadata() {
        // Use a dummy SessionManager for metadata tests.
        // The Arc is only needed for construction; metadata doesn't use it.
        let sm = Arc::new(SessionManager::new(
            &closeclaw_gateway::GatewayConfig::default(),
            None,
            None,
            closeclaw_session::persistence::ReasoningLevel::default(),
        ));
        let tool = SessionsYieldTool::new(sm);
        assert_eq!(tool.name(), "sessions_yield");
        assert_eq!(tool.group(), "sessions");
        assert!(tool.flags().is_concurrency_safe);
        assert!(!tool.flags().is_read_only);
    }

    #[test]
    fn test_sessions_yield_input_schema() {
        let sm = Arc::new(SessionManager::new(
            &closeclaw_gateway::GatewayConfig::default(),
            None,
            None,
            closeclaw_session::persistence::ReasoningLevel::default(),
        ));
        let tool = SessionsYieldTool::new(sm);
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        // No required fields
        assert!(schema.get("required").is_none());
    }
}
