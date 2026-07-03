//! Registry population: wiring AgentRegistry, SkillRegistry, ToolRegistry,
//! and ConfigHotReload during daemon startup.

use crate::config_watcher;
use closeclaw_config::ConfigManager;
use closeclaw_gateway::SessionManager;
use closeclaw_gateway::SpawnController;
use closeclaw_permission::PermissionEngine;
use closeclaw_skills::DiskSkillRegistry;
use closeclaw_tools::{
    CoreToolsRegistrar, SessionToolsRegistrar, SkillsToolsRegistrar, ToolRegistrar, ToolRegistry,
};
use std::path::Path;
use std::sync::{Arc, RwLock};

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
    /// Tool registry to be wired.
    pub tool_registry: &'a Arc<ToolRegistry>,
    /// Session manager to receive config references.
    pub session_manager: &'a Arc<SessionManager>,
    /// Permission engine for builtin tool context.
    pub permission_engine: &'a Arc<PermissionEngine>,
    /// SpawnController for validating agent spawn requests.
    pub spawn_controller: Arc<SpawnController>,
    /// Path to the config subdirectory (for hot-reload).
    pub config_subdir: &'a Path,
}

/// Populate registries and wire them together.
///
/// Returns an optional [`ConfigWatcherHandle`] for config hot-reload.
pub(crate) async fn populate_registries(
    ctx: &RegistryContext<'_>,
) -> Option<config_watcher::ConfigWatcherHandle> {
    let disk_reg = acquire_disk_registry(ctx.skill_registry)?;
    load_and_populate_agents(ctx, &disk_reg);
    inject_agent_registry_into_skill_registry(ctx.skill_registry, ctx.agent_registry);
    inject_agent_registry_into_tool_registry(ctx.tool_registry, ctx.agent_registry);
    wire_session_manager(ctx).await;
    let config_watcher = init_config_hot_reload(ctx);
    spawn_builtin_tools(ctx, &disk_reg).await;
    config_watcher
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
            Arc::clone(agent_registry) as Arc<dyn closeclaw_common::AgentSkillsQuery>
        );
    }
}

/// Inject AgentRegistry into ToolRegistry.
fn inject_agent_registry_into_tool_registry(
    tool_registry: &Arc<ToolRegistry>,
    agent_registry: &Arc<closeclaw_agent::registry::AgentRegistry>,
) {
    tool_registry.set_agent_tools_query(
        Arc::clone(agent_registry) as Arc<dyn closeclaw_common::AgentToolsConfigQuery>
    );
}

/// Wire ConfigManager and AgentRegistry into SessionManager.
async fn wire_session_manager(ctx: &RegistryContext<'_>) {
    ctx.session_manager
        .set_config_manager(Arc::clone(ctx.config_manager))
        .await;
    ctx.session_manager
        .set_agent_registry(Arc::clone(ctx.agent_registry) as Arc<dyn closeclaw_common::AgentLookup>)
        .await;
}

/// Initialize config hot-reload watcher.
fn init_config_hot_reload(
    ctx: &RegistryContext<'_>,
) -> Option<config_watcher::ConfigWatcherHandle> {
    match config_watcher::init_config_hot_reload(
        &ctx.config_subdir.to_string_lossy(),
        Arc::clone(ctx.config_manager),
        Arc::clone(ctx.agent_registry),
        Arc::clone(ctx.session_manager),
    ) {
        Ok(handle) => Some(handle),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "failed to initialize config hot-reload — \
                 config changes will require restart"
            );
            None
        }
    }
}

/// Register builtin tools via the Registrar pattern.
///
/// Constructs all four standard registrars, collects them into a
/// `Vec<Box<dyn ToolRegistrar>>`, and calls
/// [`ToolRegistry::register_all`] to register everything and freeze
/// the registry.
async fn spawn_builtin_tools(ctx: &RegistryContext<'_>, disk_reg: &Arc<DiskSkillRegistry>) {
    let task_manager = Arc::new(closeclaw_tasks::BackgroundTaskManager::new());

    let core_registrar = CoreToolsRegistrar::new(
        Arc::clone(ctx.permission_engine),
        task_manager as Arc<dyn closeclaw_common::TaskManager>,
        Arc::clone(ctx.session_manager),
        Arc::clone(ctx.config_manager),
    );
    let session_registrar = SessionToolsRegistrar::new(
        Arc::clone(&ctx.spawn_controller) as Arc<dyn closeclaw_tools::SpawnValidator>,
        Arc::clone(ctx.session_manager),
        Arc::clone(ctx.agent_registry) as Arc<dyn closeclaw_common::AgentConfigLookup>,
        Arc::clone(ctx.permission_engine),
    );
    let skills_registrar = SkillsToolsRegistrar::new(
        Arc::clone(disk_reg),
        Arc::clone(&ctx.spawn_controller) as Arc<dyn closeclaw_tools::SpawnValidator>,
        Arc::clone(ctx.session_manager),
    );
    let im_adapter_registrar = closeclaw_im_adapter::ImAdapterToolsRegistrar::new();

    let registrars: Vec<Box<dyn ToolRegistrar>> = vec![
        Box::new(core_registrar),
        Box::new(session_registrar),
        Box::new(skills_registrar),
        Box::new(im_adapter_registrar),
    ];

    if let Err(e) = ctx.tool_registry.register_all(registrars).await {
        tracing::error!(error = %e, "failed to register builtin tools via registrars");
    }
}
