//! sessions_yield tool — signals the session to enter Waiting state.

use super::SessionManagerOps;
use closeclaw_agent::AgentConfigLookup;
use closeclaw_common::tool_trait::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Default yield timeout in seconds (10 minutes).
///
/// Used when no child sessions provide an explicit timeout.
/// This is the tool-level default; the gateway may apply
/// a different value via its own `DEFAULT_YIELD_TIMEOUT_SECS`.
const DEFAULT_YIELD_TIMEOUT_SECS: u64 = 600;

/// Tool that signals the current session to enter active Waiting state.
pub struct SessionsYieldTool {
    session_manager: Arc<dyn SessionManagerOps>,
    agent_config_lookup: Arc<dyn AgentConfigLookup>,
}

impl SessionsYieldTool {
    /// Create a new `SessionsYieldTool`.
    pub fn new(
        session_manager: Arc<dyn SessionManagerOps>,
        agent_config_lookup: Arc<dyn AgentConfigLookup>,
    ) -> Self {
        Self {
            session_manager,
            agent_config_lookup,
        }
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

        // Resolve timeout_warning per-child using each child's agent config.
        // Design doc: timeout_warning resolves via spawn args → target agent config → global default.
        // Each child's resolved values are stored in ChildSessionInfo during spawn.
        // When no child provides explicit values, fall back to parent agent config.
        let (timeout_warning, notify_interval_ratio) = if children.is_empty() {
            // No children: use parent agent config as fallback.
            match self
                .agent_config_lookup
                .lookup_agent_config(&ctx.agent_id)
                .await
            {
                Some(info) => (info.timeout_warning, info.timeout_notify_interval_ratio),
                None => (None, None),
            }
        } else {
            // Use the first child's resolved timeout_warning (all children spawned
            // under the same parent share the yield timeout timer).
            let first = &children[0];
            (
                first.timeout_warning_secs,
                first.timeout_notify_interval_ratio,
            )
        };

        self.session_manager
            .clone()
            .start_yield_timeout(
                session_id,
                &ctx.agent_id,
                overall,
                timeout_warning,
                notify_interval_ratio,
            )
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
