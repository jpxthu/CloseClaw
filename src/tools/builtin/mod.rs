//! Built-in tools module.
//!
//! Re-exports all builtin tool implementations and provides a single
//! registration entry point.

pub mod coding_agent;
pub mod file_ops;
pub mod git_ops;
pub mod permission;
pub mod search;
pub mod skill_creator;
pub mod skill_tool;

pub use coding_agent::CodingAgentTool;
pub use file_ops::{EditTool, GrepTool, LsTool, ReadTool, WriteTool};
pub use git_ops::{GitCommitTool, GitLogTool, GitPullTool, GitPushTool, GitStatusTool};
pub use permission::PermissionQueryTool;
pub use search::ToolSearchTool;
pub use skill_creator::SkillCreatorTool;
pub use skill_tool::SkillTool;

use std::sync::Arc;

use crate::skills::DiskSkillRegistry;

/// Registers all built-in tools with the given registry.
///
/// Currently registers 15 tools:
///
/// 5 file_ops tools:
/// - [`ReadTool`]
/// - [`WriteTool`]
/// - [`EditTool`]
/// - [`GrepTool`]
/// - [`LsTool`]
///
/// 2 meta tools:
/// - [`ToolSearchTool`]
/// - [`PermissionQueryTool`]
///
/// 5 git_ops tools:
/// - [`GitStatusTool`]
/// - [`GitLogTool`]
/// - [`GitCommitTool`]
/// - [`GitPushTool`]
/// - [`GitPullTool`]
///
/// 3 stub tools:
/// - [`CodingAgentTool`]
/// - [`SkillCreatorTool`]
/// - [`SkillTool`]
pub async fn register_builtin_tools(
    registry: &crate::tools::ToolRegistry,
    disk_registry: Arc<DiskSkillRegistry>,
) {
    // file_ops
    registry.register(ReadTool::new()).await.ok();
    registry.register(WriteTool::new()).await.ok();
    registry.register(EditTool::new()).await.ok();
    registry.register(GrepTool::new()).await.ok();
    registry.register(LsTool::new()).await.ok();
    // meta
    registry.register(ToolSearchTool::new()).await.ok();
    registry.register(PermissionQueryTool::new()).await.ok();
    // git_ops
    registry.register(GitStatusTool::new()).await.ok();
    registry.register(GitLogTool::new()).await.ok();
    registry.register(GitCommitTool::new()).await.ok();
    registry.register(GitPushTool::new()).await.ok();
    registry.register(GitPullTool::new()).await.ok();
    // stub
    registry.register(CodingAgentTool::new()).await.ok();
    registry.register(SkillCreatorTool::new()).await.ok();
    // skills
    registry.register(SkillTool::new(disk_registry)).await.ok();
}
