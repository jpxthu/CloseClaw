//! Built-in tools module.
//!
//! Re-exports all builtin tool implementations. Registration is
//! handled by the individual [`ToolRegistrar`] implementations in
//! `crate::registrars`.

pub(crate) mod approval_utils;
pub mod bash;
pub mod bash_classify;
pub mod bash_kill;
pub mod coding_agent;
pub mod execute_plan;
pub mod file_ops;
pub mod git_ops;
pub mod permission;
pub mod progress;
pub mod prompt_template;
pub mod search;
pub mod skill_creator;
pub mod skill_tool;

pub use bash::BashTool;
pub use coding_agent::CodingAgentTool;
pub use execute_plan::ExecutePlanTool;
pub use file_ops::{EditTool, GrepTool, LsTool, ReadTool, WriteTool};
pub use git_ops::{GitCommitTool, GitLogTool, GitPullTool, GitPushTool, GitStatusTool};
pub use permission::PermissionQueryTool;
pub use progress::ProgressTool;
pub use search::ToolSearchTool;
pub use skill_creator::SkillCreatorTool;
pub use skill_tool::SkillTool;

#[cfg(test)]
mod execute_plan_tests;

#[cfg(test)]
mod prompt_template_tests;

#[cfg(test)]
mod progress_tests;

#[cfg(test)]
mod skill_creator_tests;

#[cfg(test)]
mod skill_tool_tests;
