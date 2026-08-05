//! sessions_yield tool — signals the session to enter Waiting state.

use super::SessionManagerOps;
use closeclaw_common::tool_trait::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Default yield timeout in seconds (10 minutes).
///
/// Used when no child sessions provide an explicit timeout.
/// Must stay in sync with the gateway crate's
/// `DEFAULT_YIELD_TIMEOUT_SECS`.
const DEFAULT_YIELD_TIMEOUT_SECS: u64 = 600;

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
         The session becomes idle — user messages and child completion \
         notifications are delivered immediately without queuing. \
         The session resumes automatically when any message arrives."
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

        let children = self.session_manager.list_children(session_id).await;
        let overall = compute_overall_timeout(&children);

        self.session_manager
            .clone()
            .start_yield_timeout(session_id, &ctx.agent_id, overall)
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

/// Compute the overall yield timeout for a set of child sessions.
///
/// Per the design doc: `max(child timeouts) + 60s buffer`.
/// When no child provides an explicit timeout, falls back to
/// [`DEFAULT_YIELD_TIMEOUT_SECS`].
///
/// This function is testable independently of the async tool call path.
pub fn compute_overall_timeout(children: &[crate::spawn::ChildSessionInfo]) -> u64 {
    children
        .iter()
        .filter_map(|c| c.timeout_secs)
        .max()
        .unwrap_or(DEFAULT_YIELD_TIMEOUT_SECS)
        + 60
}
