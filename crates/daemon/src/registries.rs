//! Registry population: wiring AgentRegistry, BuiltinSkillRegistry, ToolRegistry,
//! and ConfigHotReload during daemon startup.

use crate::config_watcher;
use crate::trait_adapters::{ApprovalFlowAdapter, PermissionEngineAdapter};
use anyhow::Context;
use closeclaw_agent::AgentConfigLookup;
use closeclaw_common::tool_registry::ToolRegistry as ToolRegistryTrait;
use closeclaw_config::ConfigManager;
use closeclaw_gateway::SpawnController;
use closeclaw_gateway::{Gateway, SessionManager};
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::PermissionEngine;
use closeclaw_session::tools::{LateBoundSessionManagerOps, SessionToolsRegistrar};
use closeclaw_skills::{BuiltinSkillRegistry, DiskSkillRegistry};
use closeclaw_tools::builtin::PlanExecConfirmFlow;
use closeclaw_tools::builtin::SkillTool;
use closeclaw_tools::{CoreToolsRegistrar, SkillsToolsRegistrar, ToolRegistrar, ToolRegistry};
use std::path::Path;
use std::sync::{Arc, RwLock};
use tokio::sync::watch;

/// Bundles all references needed by [`populate_registries`].
///
/// Keeps the parameter count at ≤6 (per CONTRIBUTING.md) while carrying
/// every dependency the population logic requires.
pub(crate) struct RegistryContext<'a> {
    /// Config manager providing agent and skill configurations.
    pub config_manager: &'a Arc<ConfigManager>,
    /// Agent registry to be populated.
    pub agent_registry: &'a Arc<closeclaw_agent::registry::AgentRegistry>,
    /// Shared skill registry handle (may or may not contain a DiskSkillRegistry).
    pub skill_registry: &'a Arc<RwLock<Option<DiskSkillRegistry>>>,
    /// Builtin skill registry — compiled-in skills, not subject to hot reload.
    pub builtin_registry: &'a Arc<BuiltinSkillRegistry>,
    /// Tool registry to be wired.
    pub tool_registry: &'a Arc<ToolRegistry>,
    /// Session manager to receive config references.
    pub session_manager: &'a Arc<SessionManager>,
    /// Permission engine for builtin tool context.
    pub permission_engine: &'a Arc<tokio::sync::RwLock<PermissionEngine>>,
    /// SpawnController for validating agent spawn requests.
    pub spawn_controller: Arc<SpawnController>,
    /// Approval flow for routing permission denials.
    pub approval_flow: &'a Arc<tokio::sync::Mutex<ApprovalFlow>>,
    /// Plan-exec confirmation flow for independent plan-execution confirmations.
    pub confirm_flow: &'a Arc<PlanExecConfirmFlow>,
    /// Late-bound session manager proxy for deferred injection.
    pub late_bound_session_manager: Arc<LateBoundSessionManagerOps>,
    /// Path to the config subdirectory (for hot-reload).
    pub config_subdir: &'a Path,
    /// Root data directory (for deriving audit log path).
    pub data_dir: &'a Path,
    /// Gateway reference for sending IM notifications on config reload failures.
    pub gateway: &'a Arc<Gateway>,
    /// Optional channel sender to signal restart-class config changes.
    pub restart_tx: Option<tokio::sync::mpsc::Sender<String>>,
}

/// Populate registries and wire them together.
///
/// Registration order matters:
/// 1. Agent & skill registries are populated from config.
/// 2. `wire_session_manager` injects config_manager and agent_registry
///    into SessionManager for session lifecycle operations.
/// 3. `spawn_builtin_tools` registers all builtin tools (including
///    session tools via `SessionToolsRegistrar`) via the Registrar
///    pattern, registers system-level tools via `register_before_freeze`,
///    and explicitly freezes the registry.
///
/// Returns a [`ConfigWatcherHandle`] for config hot-reload.
/// Propagates errors when hot-reload initialization fails.
pub(crate) async fn populate_registries(
    ctx: &RegistryContext<'_>,
) -> anyhow::Result<config_watcher::ConfigWatcherHandle> {
    let disk_reg = match acquire_disk_registry(ctx.skill_registry) {
        Some(dr) => dr,
        None => {
            return Err(anyhow::anyhow!(
                "populate_registries: DiskSkillRegistry not available"
            ));
        }
    };
    load_and_populate_agents(ctx, &disk_reg);
    inject_agent_registry_into_skill_registry(ctx.skill_registry, ctx.agent_registry);
    inject_agent_registry_into_tool_registry(ctx.tool_registry, ctx.agent_registry);
    wire_session_manager(ctx).await;
    let config_watcher = init_config_hot_reload(ctx)?;
    spawn_builtin_tools(ctx, &disk_reg).await;
    Ok(config_watcher)
}

/// Acquire the DiskSkillRegistry from the shared handle, if available.
fn acquire_disk_registry(
    skill_registry: &Arc<RwLock<Option<DiskSkillRegistry>>>,
) -> Option<Arc<DiskSkillRegistry>> {
    let guard = skill_registry.read().unwrap();
    guard.as_ref().map(|dr| Arc::new(dr.clone()))
}

/// Load agent configs from ConfigManager and populate AgentRegistry.
fn load_and_populate_agents(ctx: &RegistryContext<'_>, _disk_reg: &DiskSkillRegistry) {
    if let Err(e) = ctx.config_manager.load_agents(None) {
        tracing::warn!(
            error = %e,
            "failed to load agent configs from ConfigManager — \
             spawn validation will use defaults"
        );
    }
    let configs: Vec<_> = ctx.config_manager.agents().into_values().collect();
    ctx.agent_registry.populate(configs);
}

/// Inject AgentRegistry into DiskSkillRegistry.
fn inject_agent_registry_into_skill_registry(
    skill_registry: &Arc<RwLock<Option<DiskSkillRegistry>>>,
    agent_registry: &Arc<closeclaw_agent::registry::AgentRegistry>,
) {
    let mut guard = skill_registry.write().unwrap();
    if let Some(ref mut disk_reg) = *guard {
        disk_reg.set_agent_skills_query(
            Arc::clone(agent_registry) as Arc<dyn closeclaw_agent::AgentSkillsQuery>
        );
    }
}

/// Inject AgentRegistry into ToolRegistry.
fn inject_agent_registry_into_tool_registry(
    tool_registry: &Arc<ToolRegistry>,
    agent_registry: &Arc<closeclaw_agent::registry::AgentRegistry>,
) {
    tool_registry.set_agent_tools_query(
        Arc::clone(agent_registry) as Arc<dyn closeclaw_agent::AgentToolsConfigQuery>
    );
}

/// Wire ConfigManager and AgentRegistry into SessionManager.
///
/// Sets the config_manager and agent_registry on `SessionManager` so
/// session lifecycle operations (restore, resolve, etc.) can access
/// agent configurations. Session tool registration is now handled
/// by `SessionToolsRegistrar` via `register_all` in `spawn_builtin_tools`.
async fn wire_session_manager(ctx: &RegistryContext<'_>) {
    ctx.session_manager
        .set_config_manager(Arc::clone(ctx.config_manager))
        .await;
    ctx.session_manager
        .set_agent_registry(
            Arc::clone(ctx.agent_registry) as Arc<dyn closeclaw_agent::AgentRegistryQuery>
        )
        .await;
}

/// Initialize config hot-reload watcher.
///
/// Propagates errors when the watcher cannot be created, preventing
/// the daemon from entering a running state without hot-reload.
fn init_config_hot_reload(
    ctx: &RegistryContext<'_>,
) -> anyhow::Result<config_watcher::ConfigWatcherHandle> {
    config_watcher::init_config_hot_reload(
        &ctx.config_subdir.to_string_lossy(),
        Arc::clone(ctx.config_manager),
        Arc::clone(ctx.agent_registry),
        Arc::clone(ctx.session_manager),
        Arc::clone(ctx.gateway),
        ctx.restart_tx.clone(),
    )
    .context("config hot-reload initialization failed")
}

/// Spawn PlanArchiveSweeper as a Layer 2 background task.
///
/// Reads the `plan_archive.threshold_days` config from ConfigManager,
/// creates a `PlanArchiveTask`, and spawns it with a shutdown channel.
/// Returns the shutdown sender and JoinHandle for use in Phase 3.
pub(crate) fn spawn_plan_archive_sweeper(
    config_manager: &ConfigManager,
    data_dir: &Path,
) -> (watch::Sender<()>, tokio::task::JoinHandle<()>) {
    let threshold_days = config_manager
        .session_config_provider()
        .map(|p| p.plan_archive_days())
        .unwrap_or(closeclaw_session::plan_archive::DEFAULT_THRESHOLD_DAYS);
    let plan_archive_task =
        closeclaw_session::background::PlanArchiveTask::new(data_dir.to_path_buf(), threshold_days);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
    let task = tokio::spawn(async move {
        plan_archive_task.run(shutdown_rx).await;
    });
    tracing::info!(
        config_dir = %data_dir.display(),
        threshold_days,
        "PlanArchiveSweeper spawned in Layer 2"
    );
    (shutdown_tx, task)
}

/// Build a `SessionToolsRegistrar` from the daemon context.
///
/// Uses trait adapters to bridge `PermissionEngine` and `ApprovalFlow`
/// into the session tool interfaces.
fn build_session_registrar(ctx: &RegistryContext<'_>) -> SessionToolsRegistrar {
    let spawn_validator: Arc<dyn closeclaw_session::spawn_validation::SpawnValidator> =
        Arc::clone(&ctx.spawn_controller)
            as Arc<dyn closeclaw_session::spawn_validation::SpawnValidator>;
    let agent_config_lookup: Arc<dyn AgentConfigLookup> =
        Arc::clone(ctx.agent_registry) as Arc<dyn AgentConfigLookup>;
    let permission_evaluator: Arc<dyn closeclaw_common::permission_types::PermissionEvaluator> =
        Arc::new(PermissionEngineAdapter(Arc::clone(ctx.permission_engine)));
    let approval_submission: Arc<
        tokio::sync::Mutex<dyn closeclaw_common::permission_types::ApprovalSubmission>,
    > = Arc::new(tokio::sync::Mutex::new(ApprovalFlowAdapter(Arc::clone(
        ctx.approval_flow,
    ))));

    SessionToolsRegistrar::new(
        spawn_validator,
        Arc::clone(&ctx.late_bound_session_manager)
            as Arc<dyn closeclaw_session::tools::SessionManagerOps>,
        agent_config_lookup,
        permission_evaluator,
        approval_submission,
    )
}

/// Register the four standard registrars via `register_all`.
///
/// Priority order per `docs/design/tools/tool-registrar.md`:
/// core(1) → session(2) → skills(3) → im_adapter(4)
///
/// Returns `Ok(())` on success; logs and returns `Err` on failure.
async fn register_standard_registrars(
    registry: &ToolRegistry,
    ctx: &RegistryContext<'_>,
    disk_reg: &Arc<DiskSkillRegistry>,
) -> anyhow::Result<()> {
    let task_manager: Arc<dyn closeclaw_tasks::TaskManager> = ctx
        .session_manager
        .get_task_manager()
        .await
        .expect("task_manager must be set on SessionManager before spawn_builtin_tools");

    let core_registrar = CoreToolsRegistrar::new(
        Arc::clone(ctx.permission_engine),
        task_manager as Arc<dyn closeclaw_tasks::TaskManager>,
        Arc::clone(ctx.session_manager),
        Arc::clone(ctx.config_manager),
        Arc::clone(ctx.approval_flow),
        Arc::clone(ctx.tool_registry)
            as Arc<dyn closeclaw_common::tool_registry::ToolRegistryQuery>,
    )
    .with_audit_log_path(ctx.data_dir.join("logs").join("audit.log"));

    let session_registrar = build_session_registrar(ctx);

    let skill_tool: Arc<dyn closeclaw_common::Tool> = Arc::new(SkillTool::new(
        Arc::clone(disk_reg),
        Arc::clone(ctx.builtin_registry),
    ));
    let skills_registrar = SkillsToolsRegistrar::new(vec![skill_tool]);
    let im_adapter_registrar = closeclaw_im_adapter::ImAdapterToolsRegistrar::new();

    let registrars: Vec<Box<dyn ToolRegistrar>> = vec![
        Box::new(core_registrar),
        Box::new(session_registrar),
        Box::new(skills_registrar),
        Box::new(im_adapter_registrar),
    ];

    registry
        .register_all(registrars)
        .await
        .map_err(|e| anyhow::anyhow!(e))
}

/// Register system-level tools (Mode + Workflow) and freeze the registry.
///
/// Mode and Workflow tools are system-level exceptions that bypass the
/// standard Registrar chain. They are registered after `register_all`
/// completes but before `freeze()`.
async fn register_system_level_tools(registry: &ToolRegistry, ctx: &RegistryContext<'_>) {
    // Mode execution trigger tool
    let mode_tool: Arc<dyn closeclaw_common::Tool> =
        Arc::new(closeclaw_tools::builtin::ModeExecutionTriggerTool::new(
            Arc::clone(ctx.session_manager),
            Arc::clone(ctx.confirm_flow),
        ));
    if let Err(e) = registry
        .register_before_freeze(mode_tool, "SystemLevel")
        .await
    {
        tracing::error!(error = %e, "failed to register ModeExecutionTrigger tool");
    }

    // Workflow tools
    let workflow_tools: Vec<Arc<dyn closeclaw_common::Tool>> = vec![
        Arc::new(closeclaw_tools::builtin::WorkflowStartTool),
        Arc::new(closeclaw_tools::builtin::WorkflowVerifyTool),
        Arc::new(closeclaw_tools::builtin::WorkflowJumpTool),
        Arc::new(closeclaw_tools::builtin::WorkflowBlockedTool),
    ];
    for tool in workflow_tools {
        if let Err(e) = registry.register_before_freeze(tool, "SystemLevel").await {
            tracing::error!(error = %e, "failed to register Workflow tool");
        }
    }

    // Freeze the registry — no further registrations accepted
    registry.freeze();
}

/// Register builtin tools via the Registrar pattern.
///
/// Delegates to [`register_standard_registrars`] for the four standard
/// registrars (core → session → skills → im_adapter), then to
/// [`register_system_level_tools`] for Mode + Workflow tools and freeze.
async fn spawn_builtin_tools(ctx: &RegistryContext<'_>, disk_reg: &Arc<DiskSkillRegistry>) {
    if let Err(e) = register_standard_registrars(ctx.tool_registry, ctx, disk_reg).await {
        tracing::error!(error = %e, "failed to register builtin tools via registrars");
        return;
    }
    register_system_level_tools(ctx.tool_registry, ctx).await;
}

#[cfg(test)]
mod tests {
    use closeclaw_tools::ToolRegistry;
    use std::sync::Arc;

    use async_trait::async_trait;
    use closeclaw_common::tool_registry::{
        ToolRegistrar, ToolRegistrarError, ToolRegistry as ToolRegistryTrait,
    };
    use closeclaw_common::tool_trait::Tool;
    use closeclaw_tools::ToolFlags;

    /// A simple test tool for registration flow tests.
    struct TestTool {
        name: String,
        group: String,
    }

    #[async_trait]
    impl Tool for TestTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn group(&self) -> &str {
            &self.group
        }
        fn summary(&self) -> String {
            self.name.clone()
        }
        fn detail(&self) -> String {
            format!("detail for {}", self.name)
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object", "properties": {} })
        }
        fn flags(&self) -> ToolFlags {
            ToolFlags::default()
        }
    }

    /// A simple registrar that registers a fixed set of tools.
    struct TestRegistrar {
        priority: i32,
        tools: Vec<Arc<dyn Tool>>,
    }

    #[async_trait]
    impl ToolRegistrar for TestRegistrar {
        fn name(&self) -> &str {
            "TestRegistrar"
        }
        fn priority(&self) -> u32 {
            self.priority as u32
        }

        async fn register(
            &self,
            registry: &dyn ToolRegistryTrait,
        ) -> Result<(), ToolRegistrarError> {
            for tool in &self.tools {
                let boxed: Box<dyn std::any::Any + Send + Sync> =
                    Box::new(closeclaw_common::tool_registry::ToolBox(tool.clone()));
                registry
                    .register_any(boxed, &format!("TestRegistrar-{}", self.priority))
                    .await
                    .map_err(|e| match e {
                        closeclaw_common::tool_registry::RegistryError::Conflict {
                            tool,
                            registrar,
                            attempting,
                        } => ToolRegistrarError::Conflict {
                            tool,
                            registrar,
                            attempting,
                        },
                        other => ToolRegistrarError::Internal(other.to_string()),
                    })?;
            }
            Ok(())
        }
    }

    /// Helper: create a TestRegistrar with the given priority and tool names.
    fn make_registrar(priority: i32, tool_names: &[(&str, &str)]) -> TestRegistrar {
        let tools: Vec<Arc<dyn Tool>> = tool_names
            .iter()
            .map(|(name, group)| {
                Arc::new(TestTool {
                    name: name.to_string(),
                    group: group.to_string(),
                }) as Arc<dyn Tool>
            })
            .collect();
        TestRegistrar { priority, tools }
    }

    /// Verify that four standard registrars are registered in priority order
    /// and the resulting ToolRegistry contains all expected tools.
    #[tokio::test]
    async fn test_registration_flow_standard_registrars() {
        let reg = ToolRegistry::new();

        // Four standard registrars: core(1), session(2), skills(3), im_adapter(4)
        let core_reg = make_registrar(
            1,
            &[
                ("Read", "file_ops"),
                ("Write", "file_ops"),
                ("Bash", "bash"),
            ],
        );
        let session_reg = make_registrar(
            2,
            &[("sessions_spawn", "session"), ("sessions_list", "session")],
        );
        let skills_reg = make_registrar(3, &[("SkillTool", "skills")]);
        let im_adapter_reg = make_registrar(4, &[]);

        let registrars: Vec<Box<dyn ToolRegistrar>> = vec![
            Box::new(im_adapter_reg), // intentionally out of order
            Box::new(core_reg),
            Box::new(skills_reg),
            Box::new(session_reg),
        ];

        reg.register_all(registrars).await.unwrap();

        // Verify all standard tools are present
        assert!(reg.get_detail("Read").await.is_ok());
        assert!(reg.get_detail("Write").await.is_ok());
        assert!(reg.get_detail("Bash").await.is_ok());
        assert!(reg.get_detail("sessions_spawn").await.is_ok());
        assert!(reg.get_detail("sessions_list").await.is_ok());
        assert!(reg.get_detail("SkillTool").await.is_ok());
    }

    /// Verify that Mode and Workflow tools can be registered via
    /// register_before_freeze after standard registrars but before freeze.
    #[tokio::test]
    async fn test_registration_flow_system_level_tools() {
        let reg = ToolRegistry::new();

        // Standard registrar
        let core_reg = make_registrar(1, &[("Read", "file_ops")]);
        reg.register_all(vec![Box::new(core_reg)]).await.unwrap();

        // System-level tools via register_before_freeze
        let mode_tool: Arc<dyn Tool> = Arc::new(TestTool {
            name: "ModeExecutionTrigger".to_string(),
            group: "mode".to_string(),
        });
        reg.register_before_freeze(mode_tool, "SystemLevel")
            .await
            .unwrap();

        let workflow_tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(TestTool {
                name: "WorkflowStart".to_string(),
                group: "workflow".to_string(),
            }),
            Arc::new(TestTool {
                name: "WorkflowVerify".to_string(),
                group: "workflow".to_string(),
            }),
            Arc::new(TestTool {
                name: "WorkflowJump".to_string(),
                group: "workflow".to_string(),
            }),
            Arc::new(TestTool {
                name: "WorkflowBlocked".to_string(),
                group: "workflow".to_string(),
            }),
        ];
        for tool in workflow_tools {
            reg.register_before_freeze(tool, "SystemLevel")
                .await
                .unwrap();
        }

        // Verify system-level tools are present
        assert!(reg.get_detail("ModeExecutionTrigger").await.is_ok());
        assert!(reg.get_detail("WorkflowStart").await.is_ok());
        assert!(reg.get_detail("WorkflowVerify").await.is_ok());
        assert!(reg.get_detail("WorkflowJump").await.is_ok());
        assert!(reg.get_detail("WorkflowBlocked").await.is_ok());
    }

    /// Verify SkillCreator is NOT registered as an independent tool.
    /// It should be dispatched via SkillTool, not registered directly.
    #[tokio::test]
    async fn test_skill_creator_not_registered_independently() {
        let reg = ToolRegistry::new();

        // Register standard registrars (skills_reg only registers SkillTool)
        let skills_reg = make_registrar(3, &[("SkillTool", "skills")]);
        reg.register_all(vec![Box::new(skills_reg)]).await.unwrap();

        // SkillCreatorTool should NOT be in the registry
        assert!(
            reg.get_detail("SkillCreator").await.is_err(),
            "SkillCreator should NOT be registered as an independent tool"
        );
    }

    /// Verify that after full registration flow, all expected tools
    /// (core + session + skills + im_adapter + mode + workflow) are present.
    #[tokio::test]
    async fn test_full_registration_flow_all_expected_tools() {
        let reg = ToolRegistry::new();

        // Standard registrars
        let core_reg = make_registrar(
            1,
            &[
                ("Read", "file_ops"),
                ("Write", "file_ops"),
                ("Bash", "bash"),
            ],
        );
        let session_reg = make_registrar(2, &[("sessions_spawn", "session")]);
        let skills_reg = make_registrar(3, &[("SkillTool", "skills")]);
        let im_adapter_reg = make_registrar(4, &[]);

        reg.register_all(vec![
            Box::new(core_reg),
            Box::new(session_reg),
            Box::new(skills_reg),
            Box::new(im_adapter_reg),
        ])
        .await
        .unwrap();

        // System-level tools
        let mode_tool: Arc<dyn Tool> = Arc::new(TestTool {
            name: "ModeExecutionTrigger".to_string(),
            group: "mode".to_string(),
        });
        reg.register_before_freeze(mode_tool, "SystemLevel")
            .await
            .unwrap();

        for name in &[
            "WorkflowStart",
            "WorkflowVerify",
            "WorkflowJump",
            "WorkflowBlocked",
        ] {
            let tool: Arc<dyn Tool> = Arc::new(TestTool {
                name: name.to_string(),
                group: "workflow".to_string(),
            });
            reg.register_before_freeze(tool, "SystemLevel")
                .await
                .unwrap();
        }

        // Freeze
        reg.freeze();

        // Verify all expected tools
        assert!(reg.get_detail("Read").await.is_ok());
        assert!(reg.get_detail("Write").await.is_ok());
        assert!(reg.get_detail("Bash").await.is_ok());
        assert!(reg.get_detail("sessions_spawn").await.is_ok());
        assert!(reg.get_detail("SkillTool").await.is_ok());
        assert!(reg.get_detail("ModeExecutionTrigger").await.is_ok());
        assert!(reg.get_detail("WorkflowStart").await.is_ok());
        assert!(reg.get_detail("WorkflowVerify").await.is_ok());
        assert!(reg.get_detail("WorkflowJump").await.is_ok());
        assert!(reg.get_detail("WorkflowBlocked").await.is_ok());

        // SkillCreator NOT present
        assert!(reg.get_detail("SkillCreator").await.is_err());
    }
}
