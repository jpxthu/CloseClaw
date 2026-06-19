//! Built-in sessions_kill tool — terminates a persistent child session and releases resources.

use crate::gateway::SessionManager;
use crate::tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Tool that kills a child session by stopping it (cascade)
/// and removing it from all tracking tables.
///
/// Works on any mode (run / session) children owned by the calling parent.
pub struct SessionsKillTool {
    session_manager: Arc<SessionManager>,
}

impl SessionsKillTool {
    /// Create a new `SessionsKillTool` with the given dependencies.
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
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
                "childId": {
                    "type": "string",
                    "description": "The session ID of the child session to kill"
                }
            },
            "required": ["childId"]
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

        // 4. Kill the child session
        self.session_manager
            .kill_child(parent_session_id, child_id)
            .await
            .map_err(ToolCallError::ExecutionFailed)?;

        // 5. Return result
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
