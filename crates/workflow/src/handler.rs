//! `/workflow` slash entry: parse-only [`WorkflowHandler`].
//!
//! Per `docs/design/workflow/workflow-tools.md`（触发机制）, the workflow
//! module provides the [`WorkflowHandler`] as the `SlashHandler`
//! implementation for the `/workflow` route. It only parses the workflow
//! `name` and returns
//! [`SlashResult::WorkflowStart`](closeclaw_common::slash_router::SlashResult::WorkflowStart)
//! — no side effects happen here. The Gateway executes the launch through
//! `WorkflowControl::start` after enforcing the Owner check, then replies
//! with the confirmation or error text.

use closeclaw_common::slash_router::{SlashContext, SlashHandler, SlashResult};

/// `/workflow <name>` — parse the workflow name, produce no side effects.
///
/// - empty / whitespace-only args → usage hint [`SlashResult::Reply`]
/// - otherwise → `SlashResult::WorkflowStart { name }` with the trimmed name
///
/// Contract: `commands() == ["workflow"]`, non-immediate, no permission
/// marker — the Owner check is enforced by the Gateway before dispatch.
#[derive(Clone, Default)]
pub struct WorkflowHandler;

impl WorkflowHandler {
    /// Create the stateless workflow slash handler.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl SlashHandler for WorkflowHandler {
    fn commands(&self) -> &[&str] {
        &["workflow"]
    }

    fn description(&self) -> &str {
        "启动受控工作流"
    }

    fn immediate(&self, _cmd: &str, _args: &str) -> bool {
        false
    }

    fn requires_permission(&self) -> bool {
        false
    }

    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        let name = args.trim();
        if name.is_empty() {
            return SlashResult::Reply("用法：/workflow <name>".to_owned());
        }
        SlashResult::WorkflowStart {
            name: name.to_owned(),
        }
    }

    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
#[path = "handler_tests.rs"]
mod tests;
