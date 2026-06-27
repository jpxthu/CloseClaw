//! Built-in tools module.
//!
//! Re-exports all builtin tool implementations and provides a single
//! registration entry point.

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

use std::sync::Arc;

use crate::{SpawnValidator, ToolRegistry};

use closeclaw_common::{AgentToolsConfigQuery, TaskManager};
use closeclaw_gateway::SessionManager;
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_skills::DiskSkillRegistry;

/// Shared dependencies for built-in tool registration.
///
/// Bundles the common `Arc` handles so that [`register_builtin_tools`]
/// stays within the 6-parameter limit defined in CONTRIBUTING.md.
///
/// Concrete types from sub-crates are used directly; main-crate types
/// (`AgentRegistry`, `SpawnController`, `BackgroundTaskManager`) are
/// abstracted via traits in `closeclaw-common`.
pub struct BuiltinToolContext {
    pub config_manager: Arc<closeclaw_config::ConfigManager>,
    /// Agent config query (trait object — implemented by AgentRegistry in main crate).
    pub agent_tools_query: Arc<dyn AgentToolsConfigQuery>,
    /// Agent config lookup (trait object — implemented by AgentRegistry in main crate).
    pub agent_config_lookup: Arc<dyn closeclaw_common::AgentConfigLookup>,
    pub disk_registry: Arc<DiskSkillRegistry>,
    pub permission_engine: Arc<PermissionEngine>,
    /// Spawn validator (trait object — implemented by SpawnController in main crate).
    pub spawn_validator: Arc<dyn SpawnValidator>,
    pub session_manager: Arc<SessionManager>,
    /// Background task manager (trait object — implemented by BackgroundTaskManager in main crate).
    pub task_manager: Arc<dyn TaskManager>,
}

/// Registers all built-in tools with the given registry.
pub async fn register_builtin_tools(registry: &ToolRegistry, context: Arc<BuiltinToolContext>) {
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
    // bash
    registry
        .register(BashTool::new(
            context.permission_engine.clone(),
            context.task_manager.clone(),
            context.session_manager.clone(),
            context.config_manager.clone(),
        ))
        .await
        .ok();
    // skills
    registry
        .register(SkillTool::new(
            context.disk_registry.clone(),
            context.spawn_validator.clone(),
            context.session_manager.clone(),
        ))
        .await
        .ok();
    // sessions
    registry
        .register(SessionsSpawnTool::new(
            context.spawn_validator.clone(),
            context.session_manager.clone(),
            context.agent_config_lookup.clone(),
        ))
        .await
        .ok();
    registry
        .register(SessionsSteerTool::new(context.session_manager.clone()))
        .await
        .ok();
    registry
        .register(SessionsKillTool::new(context.session_manager.clone()))
        .await
        .ok();
}

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
