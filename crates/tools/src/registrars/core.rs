//! Core tools registrar — file_ops, meta, git_ops, bash groups.
//!
//! Registers 14 built-in tools that belong to the core domain.

use async_trait::async_trait;
use std::sync::Arc;

use closeclaw_common::audit_log::AuditLogger;
use closeclaw_common::TaskManager;

use crate::builtin::read_truncator::{ReadTruncationProvider, TruncationConfig};
use crate::builtin::{
    AuditLogTool, BashTool, EditTool, GitCommitTool, GitLogTool, GitPullTool, GitPushTool,
    GitStatusTool, GrepTool, LsTool, PermissionQueryTool, ReadTool, ToolSearchTool, WriteTool,
};
use crate::permission_check::PermDeps;
use crate::plan_file_access::PlanFileAccess;
use crate::try_register;
use crate::Tool;
use closeclaw_common::tool_registry::{ToolRegistrar, ToolRegistrarError, ToolRegistryQuery};

/// Core tools registrar — registers all tools from the core domain.
///
/// Covers `file_ops`, `meta`, `git_ops`, and `bash` groups (14 tools).
pub struct CoreToolsRegistrar {
    permission_check: PermDeps,
    task_manager: Arc<dyn TaskManager>,
    tool_registry: Arc<dyn ToolRegistryQuery>,
    audit_logger: Option<Arc<dyn AuditLogger>>,
    read_truncation: ReadTruncationProvider,
    plan_file_access: Option<Arc<dyn PlanFileAccess>>,
}

impl CoreToolsRegistrar {
    /// Create a new `CoreToolsRegistrar` with the required dependencies.
    pub fn new(
        permission_check: PermDeps,
        task_manager: Arc<dyn TaskManager>,
        tool_registry: Arc<dyn ToolRegistryQuery>,
    ) -> Self {
        Self {
            permission_check,
            task_manager,
            tool_registry,
            audit_logger: None,
            read_truncation: Arc::new(TruncationConfig::default),
            plan_file_access: None,
        }
    }

    /// Set the audit logger for the `AuditLog` tool.
    pub fn with_audit_logger(mut self, logger: Arc<dyn AuditLogger>) -> Self {
        self.audit_logger = Some(logger);
        self
    }

    /// Set the Read tool truncation config provider.
    ///
    /// Invoked on every Read call so config hot-reloads take effect.
    /// When not set, default truncation thresholds are used.
    pub fn with_read_truncation(mut self, provider: ReadTruncationProvider) -> Self {
        self.read_truncation = provider;
        self
    }

    /// Set the plan-file access port used by Write/Edit for `plans/`
    /// writes.
    ///
    /// Without a port, Write/Edit calls targeting `plans/` paths fail
    /// fast instead of silently dropping the session's plan sidecar
    /// lock and atomic-write protection; non-plans paths are unaffected.
    pub fn with_plan_file_access(mut self, plan_file_access: Arc<dyn PlanFileAccess>) -> Self {
        self.plan_file_access = Some(plan_file_access);
        self
    }
}

/// Wire the registrar's plan-file port into a tool under construction,
/// or return the tool unwired when no port is set. `attach` is the
/// tool's own `with_plan_file_access` builder.
fn attach_plan_port<T>(
    tool: T,
    port: Option<&Arc<dyn PlanFileAccess>>,
    attach: impl FnOnce(T, Arc<dyn PlanFileAccess>) -> T,
) -> T {
    match port {
        Some(port) => attach(tool, Arc::clone(port)),
        None => tool,
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
            ReadTool::new(Arc::clone(&self.read_truncation)),
            r
        );
        let write_tool = attach_plan_port(
            WriteTool::new(),
            self.plan_file_access.as_ref(),
            WriteTool::with_plan_file_access,
        );
        let edit_tool = attach_plan_port(
            EditTool::new(),
            self.plan_file_access.as_ref(),
            EditTool::with_plan_file_access,
        );
        try_register!(registry, registered, write_tool, r);
        try_register!(registry, registered, edit_tool, r);
        try_register!(registry, registered, GrepTool::new(), r);
        try_register!(registry, registered, LsTool::new(), r);
        try_register!(
            registry,
            registered,
            ToolSearchTool::new(Arc::clone(&self.tool_registry)),
            r
        );
        try_register!(registry, registered, PermissionQueryTool::new(), r);
        try_register!(registry, registered, GitStatusTool::new(), r);
        try_register!(registry, registered, GitLogTool::new(), r);
        try_register!(registry, registered, GitCommitTool::new(), r);
        try_register!(registry, registered, GitPushTool::new(), r);
        try_register!(registry, registered, GitPullTool::new(), r);
        // audit_log (optional — requires an injected audit logger)
        if let Some(ref logger) = self.audit_logger {
            try_register!(
                registry,
                registered,
                AuditLogTool::new(Arc::clone(logger)),
                r
            );
        }
        // bash
        try_register!(
            registry,
            registered,
            BashTool::new(
                Arc::clone(&self.permission_check),
                Arc::clone(&self.task_manager),
            ),
            r
        );
        if registered == 0 {
            return Err(ToolRegistrarError::Internal(
                "all 14 tools failed to register".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
