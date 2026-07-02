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
use crate::registrar::{ToolRegistrar, ToolRegistrarError};
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
        register_tool(registry, ReadTool::new()).await?;
        register_tool(registry, WriteTool::new()).await?;
        register_tool(registry, EditTool::new()).await?;
        register_tool(registry, GrepTool::new()).await?;
        register_tool(registry, LsTool::new()).await?;
        // meta
        register_tool(registry, ToolSearchTool::new()).await?;
        register_tool(registry, PermissionQueryTool::new()).await?;
        // git_ops
        register_tool(registry, GitStatusTool::new()).await?;
        register_tool(registry, GitLogTool::new()).await?;
        register_tool(registry, GitCommitTool::new()).await?;
        register_tool(registry, GitPushTool::new()).await?;
        register_tool(registry, GitPullTool::new()).await?;
        // stub (tools-internal, not listed in design doc)
        register_tool(registry, CodingAgentTool::new()).await?;
        // bash
        register_tool(
            registry,
            BashTool::new(
                self.permission_engine.clone(),
                self.task_manager.clone(),
                self.session_manager.clone(),
                self.config_manager.clone(),
            ),
        )
        .await?;

        Ok(())
    }
}

/// Helper: register a single tool, converting `ToolError` into `ToolRegistrarError`.
async fn register_tool(
    registry: &ToolRegistry,
    tool: impl crate::Tool + 'static,
) -> Result<(), ToolRegistrarError> {
    registry.register(tool).await.map_err(|e| match e {
        crate::ToolError::AlreadyRegistered(name) => ToolRegistrarError::Conflict {
            tool: name,
            registrar: "CoreToolsRegistrar".to_string(),
        },
        other => ToolRegistrarError::Internal(other.to_string()),
    })
}
