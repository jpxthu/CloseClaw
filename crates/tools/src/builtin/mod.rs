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
pub mod file_ops;
pub mod git_ops;
pub mod permission;
pub mod prompt_template;
pub mod search;
pub mod sessions_kill;
pub mod sessions_spawn;
pub mod sessions_steer;
pub mod skill_creator;
pub mod skill_tool;

pub use bash::BashTool;
pub use coding_agent::CodingAgentTool;
pub use file_ops::{EditTool, GrepTool, LsTool, ReadTool, WriteTool};
pub use git_ops::{GitCommitTool, GitLogTool, GitPullTool, GitPushTool, GitStatusTool};
pub use permission::PermissionQueryTool;
pub use search::ToolSearchTool;
pub use sessions_kill::SessionsKillTool;
pub use sessions_spawn::SessionsSpawnTool;
pub use sessions_steer::SessionsSteerTool;
pub use skill_creator::SkillCreatorTool;
pub use skill_tool::SkillTool;

#[cfg(test)]
mod skill_tool_tests;

#[cfg(test)]
#[path = "sessions_spawn_tests.rs"]
mod sessions_spawn_tests;

#[cfg(test)]
#[path = "sessions_spawn_permission_tests.rs"]
mod sessions_spawn_permission_tests;

#[cfg(test)]
#[path = "sessions_steer_kill_tests.rs"]
mod sessions_steer_kill_tests;

#[cfg(test)]
#[path = "bash_approval_tests.rs"]
mod bash_approval_tests;

#[cfg(test)]
#[path = "sessions_kill_approval_tests.rs"]
mod sessions_kill_approval_tests;

#[cfg(test)]
#[path = "sessions_steer_approval_tests.rs"]
mod sessions_steer_approval_tests;

#[cfg(test)]
#[path = "sessions_spawn_approval_tests.rs"]
mod sessions_spawn_approval_tests;
