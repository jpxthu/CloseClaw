//! Core tools registrar — file_ops, meta, git_ops, bash groups.
//!
//! Registers 14 built-in tools that belong to the core domain.

use async_trait::async_trait;
use std::sync::Arc;

use closeclaw_common::TaskManager;
use closeclaw_config::ConfigManager;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::engine::engine_eval::PermissionEngine;

use crate::builtin::{
    BashTool, CodingAgentTool, EditTool, GitCommitTool, GitLogTool, GitPullTool, GitPushTool,
    GitStatusTool, GrepTool, LsTool, PermissionQueryTool, ReadTool, ToolSearchTool, WriteTool,
};
use crate::registrar::{register_tool, ToolRegistrar, ToolRegistrarError};
use crate::ToolRegistry;

/// Core tools registrar — registers all tools from the core domain.
///
/// Covers `file_ops`, `meta`, `git_ops`, and `bash` groups (14 tools).
pub struct CoreToolsRegistrar {
    permission_engine: Arc<PermissionEngine>,
    task_manager: Arc<dyn TaskManager>,
    session_manager: Arc<SessionManager>,
    config_manager: Arc<ConfigManager>,
}

impl CoreToolsRegistrar {
    /// Create a new `CoreToolsRegistrar` with the required dependencies.
    pub fn new(
        permission_engine: Arc<PermissionEngine>,
        task_manager: Arc<dyn TaskManager>,
        session_manager: Arc<SessionManager>,
        config_manager: Arc<ConfigManager>,
    ) -> Self {
        Self {
            permission_engine,
            task_manager,
            session_manager,
            config_manager,
        }
    }
}

#[async_trait]
impl ToolRegistrar for CoreToolsRegistrar {
    fn name(&self) -> &str {
        "CoreToolsRegistrar"
    }

    fn priority(&self) -> u32 {
        1
    }

    async fn register(&self, registry: &ToolRegistry) -> Result<(), ToolRegistrarError> {
        // file_ops
        register_tool(registry, ReadTool::new(), "CoreToolsRegistrar").await?;
        register_tool(registry, WriteTool::new(), "CoreToolsRegistrar").await?;
        register_tool(registry, EditTool::new(), "CoreToolsRegistrar").await?;
        register_tool(registry, GrepTool::new(), "CoreToolsRegistrar").await?;
        register_tool(registry, LsTool::new(), "CoreToolsRegistrar").await?;
        // meta
        register_tool(registry, ToolSearchTool::new(), "CoreToolsRegistrar").await?;
        register_tool(registry, PermissionQueryTool::new(), "CoreToolsRegistrar").await?;
        // git_ops
        register_tool(registry, GitStatusTool::new(), "CoreToolsRegistrar").await?;
        register_tool(registry, GitLogTool::new(), "CoreToolsRegistrar").await?;
        register_tool(registry, GitCommitTool::new(), "CoreToolsRegistrar").await?;
        register_tool(registry, GitPushTool::new(), "CoreToolsRegistrar").await?;
        register_tool(registry, GitPullTool::new(), "CoreToolsRegistrar").await?;
        // stub (tools-internal, not listed in design doc)
        register_tool(registry, CodingAgentTool::new(), "CoreToolsRegistrar").await?;
        // bash
        register_tool(
            registry,
            BashTool::new(
                self.permission_engine.clone(),
                self.task_manager.clone(),
                self.session_manager.clone(),
                self.config_manager.clone(),
            ),
            "CoreToolsRegistrar",
        )
        .await?;

        Ok(())
    }
}
