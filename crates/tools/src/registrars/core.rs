//! Core tools registrar — file_ops, meta, git_ops, bash groups.
//!
//! Registers 14 built-in tools that belong to the core domain.

use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

use closeclaw_config::ConfigManager;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_tasks::TaskManager;

use crate::builtin::{
    AuditLogTool, BashTool, CodingAgentTool, EditTool, GitCommitTool, GitLogTool, GitPullTool,
    GitPushTool, GitStatusTool, GrepTool, LsTool, PermissionQueryTool, ReadTool, ToolSearchTool,
    WriteTool,
};
use crate::try_register;
use crate::Tool;
use closeclaw_common::tool_registry::{ToolRegistrar, ToolRegistrarError};

/// Core tools registrar — registers all tools from the core domain.
///
/// Covers `file_ops`, `meta`, `git_ops`, and `bash` groups (15 tools).
pub struct CoreToolsRegistrar {
    permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
    task_manager: Arc<dyn TaskManager>,
    session_manager: Arc<SessionManager>,
    config_manager: Arc<ConfigManager>,
    approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
    audit_log_path: Option<PathBuf>,
}

impl CoreToolsRegistrar {
    /// Create a new `CoreToolsRegistrar` with the required dependencies.
    pub fn new(
        permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
        task_manager: Arc<dyn TaskManager>,
        session_manager: Arc<SessionManager>,
        config_manager: Arc<ConfigManager>,
        approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
    ) -> Self {
        Self {
            permission_engine,
            task_manager,
            session_manager,
            config_manager,
            approval_flow,
            audit_log_path: None,
        }
    }

    /// Set the audit log path for the `AuditLog` tool.
    pub fn with_audit_log_path(mut self, path: PathBuf) -> Self {
        self.audit_log_path = Some(path);
        self
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

    async fn register(
        &self,
        registry: &dyn closeclaw_common::tool_registry::ToolRegistry,
    ) -> Result<(), ToolRegistrarError> {
        let mut registered = 0usize;
        let r = self.name();
        try_register!(
            registry,
            registered,
            ReadTool::new(
                self.permission_engine.clone(),
                self.session_manager.clone(),
                self.config_manager.clone(),
                self.approval_flow.clone(),
            ),
            r
        );
        try_register!(
            registry,
            registered,
            WriteTool::new(
                self.permission_engine.clone(),
                self.session_manager.clone(),
                self.config_manager.clone(),
                self.approval_flow.clone(),
            ),
            r
        );
        try_register!(
            registry,
            registered,
            EditTool::new(
                self.permission_engine.clone(),
                self.session_manager.clone(),
                self.config_manager.clone(),
                self.approval_flow.clone(),
            ),
            r
        );
        try_register!(
            registry,
            registered,
            GrepTool::new(
                self.permission_engine.clone(),
                self.session_manager.clone(),
                self.config_manager.clone(),
                self.approval_flow.clone(),
            ),
            r
        );
        try_register!(
            registry,
            registered,
            LsTool::new(
                self.permission_engine.clone(),
                self.session_manager.clone(),
                self.config_manager.clone(),
                self.approval_flow.clone(),
            ),
            r
        );
        try_register!(registry, registered, ToolSearchTool::new(), r);
        try_register!(registry, registered, PermissionQueryTool::new(), r);
        try_register!(registry, registered, GitStatusTool::new(), r);
        try_register!(registry, registered, GitLogTool::new(), r);
        try_register!(registry, registered, GitCommitTool::new(), r);
        try_register!(registry, registered, GitPushTool::new(), r);
        try_register!(registry, registered, GitPullTool::new(), r);
        try_register!(registry, registered, CodingAgentTool::new(), r);
        // audit_log (optional — requires audit_log_path)
        if let Some(ref path) = self.audit_log_path {
            try_register!(registry, registered, AuditLogTool::new(path.clone()), r);
        }
        // bash
        try_register!(
            registry,
            registered,
            BashTool::new(
                self.permission_engine.clone(),
                self.task_manager.clone(),
                self.session_manager.clone(),
                self.config_manager.clone(),
                self.approval_flow.clone(),
            ),
            r
        );
        if registered == 0 {
            return Err(ToolRegistrarError::Internal(
                "all 15 tools failed to register".to_string(),
            ));
        }
        Ok(())
    }
}
