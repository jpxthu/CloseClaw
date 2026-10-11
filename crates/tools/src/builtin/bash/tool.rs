use crate::permission_check::PermDeps;
use crate::{PromptGenerationContext, Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use super::exec::execute_bash_call;
use super::prompt::{
    prompt_add_combination_suggestions, prompt_add_permission_status, prompt_add_workdir_guidance,
    prompt_usage_principles, prompt_when_to_use,
};

/// Shell command execution tool with timeout, output control, and auto-backgroundize.
pub struct BashTool {
    permission_check: PermDeps,
    bg_manager: Arc<dyn closeclaw_common::TaskManager>,
}

impl BashTool {
    /// Creates a new `BashTool` backed by the given permission check
    /// port and background task manager.
    pub fn new(
        permission_check: PermDeps,
        bg_manager: Arc<dyn closeclaw_common::TaskManager>,
    ) -> Self {
        Self {
            permission_check,
            bg_manager,
        }
    }

    /// Bundle permission dependencies into a [`PermDeps`] port handle.
    fn perm_deps(&self) -> PermDeps {
        Arc::clone(&self.permission_check)
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn group(&self) -> &str {
        "bash"
    }

    fn summary(&self) -> String {
        "Execute shell commands with timeout and output control".to_string()
    }

    fn detail(&self) -> String {
        "[keywords: bash shell command run execute script terminal] \
         Execute a shell command via subprocess. Supports timeout control \
         (default 120s, max 600s), output truncation with head-preservation \
         (threshold 30,000 chars), and output persistence to disk when \
         output exceeds threshold. Supports run_in_background for async \
         execution. Commands exceeding 15s are auto-backgrounded. \
         Background tasks notify automatically on completion - do not poll. \
         Use run_in_background for commands expected to exceed 10 seconds."
            .to_string()
    }

    fn generate_prompt(&self, context: &PromptGenerationContext) -> String {
        let mut parts = Vec::new();

        parts.push(prompt_when_to_use());
        prompt_add_workdir_guidance(context, &mut parts);
        prompt_add_permission_status(context, &mut parts);
        parts.push(prompt_usage_principles());
        prompt_add_combination_suggestions(context, &mut parts);

        parts.join("\n\n")
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                },
                "timeout": {
                    "type": "number",
                    "description": "Timeout in milliseconds (default 120000, max 600000)"
                },
                "description": {
                    "type": "string",
                    "description": "Brief description of what this command does"
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Run command in background, returns task ID immediately"
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory (defaults to session workdir)"
                },
                "dangerouslyDisableSandbox": {
                    "type": "boolean",
                    "description": "Bypass sandbox restrictions (landlock + seccomp) for this command. Sandbox infrastructure is ready but enforcement strategy is implemented in a follow-up PR."
                }
            },
            "required": ["command"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_destructive: true,
            is_expensive: true,
            ..Default::default()
        }
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        execute_bash_call(&self.perm_deps(), &self.bg_manager, args, ctx).await
    }
}
