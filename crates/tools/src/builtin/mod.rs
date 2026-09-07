//! Built-in tools module.
//!
//! Re-exports all builtin tool implementations. Registration is
//! handled by the individual [`ToolRegistrar`] implementations in
//! `crate::registrars`.

pub(crate) mod approval_utils;
pub mod audit_log;
pub mod bash;
pub mod bash_classify;
pub mod bash_kill;
pub mod coding_agent;
pub mod edit_match;
pub mod file_ops;
pub mod git_ops;
pub mod mode_execution_trigger;
pub mod permission;
pub mod plan_exec_confirm;
pub mod prompt_template;
pub(crate) mod read_truncator;
pub mod search;
pub mod skill_tool;
pub mod workflow_tools;

#[cfg(test)]
pub(crate) mod test_helpers;

pub use audit_log::AuditLogTool;
pub use bash::BashTool;
pub use coding_agent::CodingAgentTool;
pub use file_ops::{EditTool, GrepTool, LsTool, ReadTool, WriteTool};
pub use git_ops::{GitCommitTool, GitLogTool, GitPullTool, GitPushTool, GitStatusTool};
pub use mode_execution_trigger::ModeExecutionTriggerTool;
pub use permission::PermissionQueryTool;
pub use plan_exec_confirm::{CreateChildSessionFn, PlanExecConfirmFlow, PlanExecNotification};
pub use search::ToolSearchTool;
pub use skill_tool::SkillTool;
pub use workflow_tools::{
    WorkflowBlockedTool, WorkflowJumpTool, WorkflowStartTool, WorkflowVerifyTool,
};

#[cfg(test)]
mod mode_execution_trigger_tests;

#[cfg(test)]
mod workflow_tools_tests;

#[cfg(test)]
mod prompt_template_tests;

#[cfg(test)]
mod read_truncator_tests;

#[cfg(test)]
mod read_tool_tests;

#[cfg(test)]
mod skill_tool_tests;
