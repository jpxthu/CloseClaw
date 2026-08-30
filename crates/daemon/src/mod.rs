//! Daemon - CloseClaw background service
//!
//! Orchestrates all components: Gateway, AgentRegistry, PermissionEngine.
//! Handles graceful shutdown via ShutdownCoordinator.
pub mod bridge;
pub mod chat_rpc;
pub mod config_helpers;
pub mod config_reload;
pub mod config_watcher;
mod daemon_struct;
pub mod dreaming_scheduler;
pub mod gateway_restart;
pub mod lifecycle;
pub mod registries;
pub mod shutdown;
pub(crate) mod shutdown_heartbeat;
pub mod skill_reload;
pub mod startup;
pub mod trait_adapters;
use crate::startup::{all_component_entries, topo_sort_layers, StartupError};
use closeclaw_cli::admin::{admin_socket_path, AdminContext, AdminServer};
use closeclaw_common::{NoopMetricsEmitter, SessionLookup};
use closeclaw_config::providers::{ConfigProvider, SystemConfigData};
use closeclaw_config::session::SessionConfigProvider;
use closeclaw_config::{ConfigManager, ConfigSection};
pub use daemon_struct::*;

/// Resolved startup plan: topo-sort layers plus validated phase components.
/// Each outer element is a layer/phase; each inner element is a [`ComponentId`].
type StartupPlan = (
    Vec<Vec<crate::startup::ComponentId>>,
    Vec<Vec<crate::startup::ComponentId>>,
);
pub use closeclaw_gateway::SpawnController;
use closeclaw_gateway::{sweeper::ArchiveSweeper, Gateway, GatewayConfig, SessionManager};
use closeclaw_memory::dreaming::DreamingPipeline;
use closeclaw_memory::miner::MemoryMiner;
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::{PermissionEngine, RuleSet};
use closeclaw_session::{
    checkpoint_manager::CheckpointManager, persistence::PersistenceService, storage::SqliteStorage,
};
use closeclaw_skills::builtin::builtin_skills;
use closeclaw_skills::{BuiltinSkillRegistry, DiskSkillRegistry};
use closeclaw_system_prompt::sections::SectionCache;
use closeclaw_tools::ToolRegistry;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::sync::watch;
use tracing::info;
mod noop_miner_llm;
mod skills_helper;
/// Parse an .env file into key-value pairs (comments, whitespace trimmed).
pub(crate) fn parse_env_file(path: &std::path::Path) -> std::io::Result<Vec<(String, String)>> {
    let content = std::fs::read_to_string(path)?;
    let mut pairs = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(pos) = line.find('=') {
            let key = line[..pos].trim().to_string();
            let value = line[pos + 1..].trim().to_string();
            if !key.is_empty() && !value.is_empty() {
                pairs.push((key, value));
            }
        }
    }
    Ok(pairs)
}
/// Load key=value pairs from a .env file and set them as env vars (lines starting with # ignored).
pub(crate) fn load_env_file(path: &std::path::Path) -> std::io::Result<()> {
    for (key, value) in parse_env_file(path)? {
        std::env::set_var(&key, &value); // load_env_file: allowed exception per CONTRIBUTING.md
    }
    Ok(())
}
mod llm_components;
mod llm_init;
#[cfg(test)]
pub mod test_helpers;
// --- Topological startup orchestration ---
impl Daemon {
    /// Resolve the deterministic startup order from the component dependency
    /// graph. Returns topo-sorted layers; errors on circular dependency.
    fn resolve_startup_order() -> Result<StartupPlan, StartupError> {
        let entries = all_component_entries();
        let layers = topo_sort_layers(&entries)?;
        let phase_components = Self::validate_phase_components(&layers)?;
        Ok((layers, phase_components))
    }

    /// Map each [`StartupPhase`] to its resolved [`ComponentId`] set,
    /// validated against the topo-sort result.
    fn validate_phase_components(
        layers: &[Vec<crate::startup::ComponentId>],
    ) -> Result<Vec<Vec<crate::startup::ComponentId>>, StartupError> {
        use crate::startup::{ComponentId, Foundation, Service};
        let c = |f: Foundation| ComponentId::Foundation(f);
        let s = |sv: Service| ComponentId::Service(sv);
        let expected: Vec<Vec<ComponentId>> = vec![
            vec![c(Foundation::ConfigManager), c(Foundation::Storage)],
            vec![
                s(Service::AgentRegistry),
                s(Service::ConfigHotReload),
                s(Service::PermissionEngine),
                s(Service::PlanArchiveSweeper),
                s(Service::RenderersPlugins),
                s(Service::SessionConfigProvider),
                s(Service::SkillsRegistry),
                s(Service::LLMRegistry),
            ],
            vec![
                s(Service::AnnounceSweeper),
                s(Service::ApprovalFlow),
                s(Service::ArchiveSweeper),
                s(Service::DreamingScheduler),
                s(Service::IMAdapters),
                s(Service::ToolsRegistry),
            ],
            vec![
                s(Service::SessionManager),
                s(Service::SpawnController),
                s(Service::SystemPromptBuilder),
            ],
            vec![s(Service::Gateway)],
            vec![s(Service::AdminRpcServer)],
        ];
        for (i, exp) in expected.iter().enumerate() {
            let mut actual = layers.get(i).cloned().unwrap_or_default();
            let mut exp_sorted = exp.clone();
            actual.sort_by_key(|id| id.name().to_string());
            exp_sorted.sort_by_key(|id| id.name().to_string());
            if actual != exp_sorted {
                return Err(StartupError::CircularDependency);
            }
        }
        Ok(expected)
    }

    /// Log the resolved startup order at `info` level for operational visibility.
    fn log_startup_order(layers: &[Vec<crate::startup::ComponentId>]) {
        for (i, layer) in layers.iter().enumerate() {
            let names: Vec<&str> = layer.iter().map(|id| id.name()).collect();
            info!(layer = i + 1, components = ?names, "startup layer resolved");
        }
    }
}

// --- Phase initialization methods ---
impl Daemon {
    /// Phase 1: Foundation — ConfigManager + Storage.
    fn init_phase_1_foundation(
        config_dir: &str,
    ) -> anyhow::Result<(Arc<ConfigManager>, Arc<SqliteStorage>, std::path::PathBuf)> {
        let config_subdir = PathBuf::from(config_dir).join("config");
        let config_manager = Arc::new(
            ConfigManager::new(config_subdir)
                .map_err(|e| anyhow::anyhow!("failed to create ConfigManager: {}", e))?,
        );
        config_manager
            .load()
            .map_err(|e| anyhow::anyhow!("failed to load mandatory config sections: {}", e))?;
        let data_dir = PathBuf::from(config_dir);
        let storage = Arc::new(
            SqliteStorage::new(&data_dir)
                .map_err(|e| anyhow::anyhow!("failed to initialize SqliteStorage: {}", e))?,
        );
        info!("SqliteStorage initialized at {}", data_dir.display());
        Self::run_config_migration(config_dir);
        Ok((config_manager, storage, data_dir))
    }

    /// Phase 2: Registries — AgentRegistry, SkillsRegistry, ToolsRegistry, LLMRegistry,
    /// PermissionEngine, PlanArchiveSweeper.
    ///
    /// Independent components within the same layer are initialized in parallel
    /// using `tokio::join!` to improve startup latency. Components with
    /// sequential dependencies (e.g. skill_registry → shared_cache) maintain
    /// their ordering.
    async fn init_phase_2_registries(
        config_dir: &str,
        config_manager: &ConfigManager,
        audit_logger: &Option<Arc<dyn closeclaw_permission::AuditLogger>>,
    ) -> anyhow::Result<(
        Arc<closeclaw_agent::registry::AgentRegistry>,
        Arc<RwLock<Option<DiskSkillRegistry>>>,
        Arc<ToolRegistry>,
        Arc<RwLock<SectionCache>>,
        Arc<dyn SessionConfigProvider>,
        Arc<closeclaw_llm::LLMRegistry>,
        Arc<closeclaw_llm::unified_fallback::UnifiedFallbackClient>,
        Arc<tokio::sync::RwLock<PermissionEngine>>,
        Option<crate::daemon_struct::PlanArchiveSweeperHandle>,
    )> {
        // Synchronous components: no async work, create directly.
        let agent_registry = Arc::new(closeclaw_agent::registry::AgentRegistry::new());
        info!("Agent registry initialized");
        let permission_engine =
            Self::build_permission_engine(config_dir, audit_logger.as_ref().cloned());
        let shared_cache = Arc::new(RwLock::new(SectionCache::new()));
        let tool_registry = Arc::new(ToolRegistry::new());
        let session_config_provider =
            config_manager.session_config_provider().unwrap_or_else(|| {
                tracing::warn!("session config provider not available after load, using defaults");
                Arc::new(
                    closeclaw_config::session::JsonSessionConfigProvider::new("/dev/null").unwrap(),
                )
            });
        let data_dir = std::path::PathBuf::from(config_dir);
        let plan_archive_sweeper =
            registries::spawn_plan_archive_sweeper(config_manager, &data_dir);

        // Parallel async components: skill_registry and llm_registry are
        // independent within Layer 2, so run them concurrently.
        let extra_dirs = skills_helper::resolve_extra_dirs(config_manager);
        let skill_fut = skill_reload::init_skill_registry(config_dir, None, extra_dirs);
        let empty_env: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        let llm_fut = Self::init_llm_registry(std::path::Path::new(config_dir), &empty_env);
        let (skill_result, (llm_registry, fallback_client)) = tokio::join!(skill_fut, llm_fut);
        let skill_registry: Arc<RwLock<Option<DiskSkillRegistry>>> = skill_result?;

        Ok((
            agent_registry,
            skill_registry,
            tool_registry,
            shared_cache,
            session_config_provider,
            llm_registry,
            fallback_client,
            permission_engine,
            plan_archive_sweeper,
        ))
    }

    /// Phase 3: Core services — Gateway, SessionManager, IM plugins, SlashDispatcher.
    async fn init_phase_3_core_services(
        config_dir: &str,
        storage: &Arc<SqliteStorage>,
        permission_engine: &Arc<tokio::sync::RwLock<PermissionEngine>>,
        config_manager: &ConfigManager,
    ) -> anyhow::Result<(
        Arc<Gateway>,
        Arc<SessionManager>,
        shutdown::ShutdownHandle,
        Vec<String>,
        Arc<closeclaw_slash::registry::HandlerRegistry>,
    )> {
        let gateway_config = GatewayConfig {
            name: "closeclaw".to_string(),
            rate_limit_per_minute: 60,
            max_message_size: 16_384,
            inbound_wal_dir: Some(std::path::PathBuf::from(config_dir).join("inbound_wal")),
            ..Default::default()
        };
        let llm_config = config_manager
            .section(ConfigSection::System)
            .and_then(|v| serde_json::from_value::<SystemConfigData>(v).ok())
            .and_then(|sys| sys.llm);
        let reasoning_level = llm_config
            .as_ref()
            .map(|llm| llm.reasoning_level)
            .unwrap_or_default();
        let session_manager = Arc::new(SessionManager::new(
            &gateway_config,
            None,
            Some(PathBuf::from(config_dir)),
            reasoning_level,
        ));
        if let Some(ref llm) = llm_config {
            if let Some(ref cache_break) = llm.cache_break {
                session_manager.set_default_cache_break_thresholds(
                    closeclaw_common::CacheBreakThresholds {
                        drop_ratio_threshold: cache_break.drop_ratio_threshold,
                        min_drop_tokens: cache_break.min_drop_tokens,
                    },
                );
            }
        }
        // Create a shared CheckpointManager for SessionManager and Gateway.
        // This unifies the persistence coordination layer (cache + storage)
        // between the two components, matching the architecture diagram.
        let storage_arc: Arc<dyn PersistenceService> =
            Arc::clone(storage) as Arc<dyn PersistenceService>;
        let checkpoint_manager = Arc::new(CheckpointManager::new(storage_arc));
        session_manager
            .set_checkpoint_manager(Arc::clone(&checkpoint_manager))
            .await;
        let gateway = Gateway::new(gateway_config, Arc::clone(&session_manager))
            .with_checkpoint_manager(Arc::clone(&checkpoint_manager));
        // Storage injection is now handled via the shared CheckpointManager
        // set on both SessionManager and Gateway above. The old
        // gateway.set_storage() path still works as a backward-compatible
        // wrapper that creates its own CheckpointManager internally.

        // Run session recovery scan: load all active checkpoints, detect
        // pending_operations, and persist recovery notifications/failure
        // results into checkpoints so resolve.rs can inject them when
        // sessions are restored.
        let dirty_sessions_for_drain: Vec<String> = {
            use closeclaw_session::recovery::SessionRecoveryService;
            let recovery_svc =
                SessionRecoveryService::new(Arc::clone(storage) as Arc<dyn PersistenceService>);
            match recovery_svc.recover().await {
                Ok(report) => {
                    if !report.dirty_sessions.is_empty() {
                        info!(
                            dirty_count = report.dirty_sessions.len(),
                            total = report.total(),
                            "recovery scan found dirty sessions"
                        );
                    } else {
                        info!(
                            total = report.total(),
                            "recovery scan complete — no dirty sessions"
                        );
                    }
                    report.dirty_sessions
                }
                Err(e) => {
                    tracing::warn!(error = %e, "recovery scan failed — continuing without recovery");
                    Vec::new()
                }
            }
        };
        if let Err(e) = session_manager.rebuild_key_registry().await {
            tracing::warn!(error = %e, "failed to rebuild key_registry — continuing");
        }
        // Startup consistency check: SQLite ↔ file system bidirectional scan.
        if let Err(e) = session_manager.run_consistency_check().await {
            tracing::warn!(error = %e, "consistency check failed — continuing");
        }
        // Mark the scan timestamp so subsequent periodic checks are incremental.
        session_manager.initialize_consistency_check_time();
        if let Err(e) = session_manager.rebuild_spawn_tree().await {
            tracing::warn!(error = %e, "failed to rebuild spawn_tree — continuing");
        }
        let gateway = Arc::new(gateway);
        gateway.set_self_ref(Arc::clone(&gateway));
        // Wire Gateway back-reference into SessionManager so
        // drain_pending_for_session can send responses via outbound pipeline.
        session_manager.set_gateway_ref(Arc::clone(&gateway)).await;
        gateway
            .set_config_dir(std::path::PathBuf::from(config_dir))
            .await;
        gateway
            .set_metrics_emitter(Arc::new(NoopMetricsEmitter))
            .await;
        closeclaw_im_adapter::platforms::register_platform_plugins(&gateway, config_dir).await;
        // Drain outbound pending messages for dirty sessions recovered earlier.
        // Each session is drained asynchronously via tokio::spawn so startup
        // is not blocked by network I/O.
        if !dirty_sessions_for_drain.is_empty() {
            let sm_ref = Arc::clone(&session_manager);
            for session_id in &dirty_sessions_for_drain {
                let sm = Arc::clone(&sm_ref);
                let session_id = session_id.clone();
                tokio::spawn(async move {
                    match sm.drain_outbound_pending_for_session(&session_id).await {
                        Ok(count) => {
                            info!(
                                session_id = %session_id,
                                delivered = count,
                                "outbound pending drain complete"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                session_id = %session_id,
                                error = %e,
                                "outbound pending drain failed"
                            );
                        }
                    }
                });
            }
        }
        Self::init_terminal_plugin(&gateway).await;
        let slash_registry =
            Self::init_slash_dispatcher(&gateway, &session_manager, permission_engine).await;
        // Start the inbound queue consumer so webhook messages are buffered.
        gateway.start_inbound_queue();
        let shutdown = shutdown::ShutdownHandle::new();
        // Wire shutdown handle into SessionManager for child-session
        // busy-count tracking during drain.
        session_manager
            .set_shutdown_handle(crate::bridge::common_shutdown_handle(&shutdown))
            .await;
        info!("Shutdown coordinator initialized");
        Ok((
            gateway,
            session_manager,
            shutdown,
            dirty_sessions_for_drain,
            slash_registry,
        ))
    }
}

/// Bundled shutdown receivers for background services.
///
/// Groups the individual `watch::Receiver<()>` arguments passed to
/// [`Daemon::spawn_background_services`] into a single struct to
/// satisfy clippy's `too_many_arguments` limit while keeping the
/// internal API ergonomic.
pub(crate) struct ServiceShutdownReceivers {
    /// Receiver for ArchiveSweeper shutdown signal.
    pub sweeper: watch::Receiver<()>,
    /// Receiver for AnnounceSweeper shutdown signal.
    pub announce_sweeper: watch::Receiver<()>,
    /// Receiver for DreamingScheduler shutdown signal.
    pub dreaming: watch::Receiver<()>,
}

// --- Phase 4-5 initialization ---
impl Daemon {
    /// Phase 4: Wiring — ApprovalFlow.
    ///
    /// ApprovalFlow and BuiltinSkillRegistry are independent within the
    /// same layer, so they are initialized in parallel via `tokio::join!`.
    async fn init_phase_4_wiring(
        gateway: &Arc<Gateway>,
        session_manager: &Arc<SessionManager>,
        permission_engine: &Arc<tokio::sync::RwLock<PermissionEngine>>,
        config_manager: &Arc<closeclaw_config::ConfigManager>,
        config_dir: &str,
        audit_logger: Option<Arc<dyn closeclaw_permission::AuditLogger>>,
    ) -> (
        Arc<tokio::sync::Mutex<ApprovalFlow>>,
        Arc<BuiltinSkillRegistry>,
    ) {
        // Build the whitelist-updated callback: invalidate the agent's cached
        // rules so the next evaluate() lazily re-reads from disk.
        let pe_clone = Arc::clone(permission_engine);
        let whitelist_cb: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |agent_id: &str| {
            if let Ok(guard) = pe_clone.try_write() {
                guard.invalidate_agent_rules(agent_id);
                tracing::info!(
                    agent = %agent_id,
                    "agent rule cache invalidated after whitelist approval"
                );
            } else {
                tracing::warn!(
                    agent = %agent_id,
                    "permission engine write lock contended, skipping cache invalidation"
                );
            }
        });

        // Build the child-session creation callback for the new-session
        // execution path.
        let sm_for_spawn = Arc::clone(session_manager);
        let cm_for_spawn = Arc::clone(config_manager);
        let create_child_fn = Self::build_create_child_fn(sm_for_spawn, cm_for_spawn);

        let mut af = ApprovalFlow::new(
            Arc::clone(session_manager) as Arc<dyn SessionLookup>,
            Arc::new(|_| {}),
            whitelist_cb,
            tokio::runtime::Handle::current(),
            HeartbeatApprovalMode::default(),
            std::path::PathBuf::from(config_dir),
            RuleSet::default(),
        );
        if let Some(logger) = audit_logger {
            af = af.with_audit_logger(logger);
        }
        af.set_create_child_session_fn(create_child_fn);
        let approval_flow = Arc::new(tokio::sync::Mutex::new(af));

        // Sync approval flow snapshot with actual loaded rules.
        {
            let pe_guard = permission_engine.read().await;
            let engine_rules = pe_guard.rules().clone();
            drop(pe_guard);
            approval_flow.lock().await.update_rules(engine_rules);
        }

        // Parallel: approval_flow wiring + builtin_skill_registry creation
        // are independent within Layer 4.
        let gw = Arc::clone(gateway);
        let af_for_gw = Arc::clone(&approval_flow);
        let approval_fut = async {
            gw.set_approval_flow(af_for_gw).await;
        };
        let builtin_fut = async {
            let skills = builtin_skills();
            let reg = Arc::new(BuiltinSkillRegistry::from_skills(skills).await);
            let count = reg.list().await.len();
            info!(count, "builtin skills registered in BuiltinSkillRegistry");
            reg
        };
        let ((), builtin_skill_registry) = tokio::join!(approval_fut, builtin_fut);

        (approval_flow, builtin_skill_registry)
    }

    /// Build the child-session creation callback for the approval flow.
    fn build_create_child_fn(
        sm: Arc<SessionManager>,
        cm: Arc<closeclaw_config::ConfigManager>,
    ) -> closeclaw_permission::approval_flow::CreateChildSessionFn {
        Arc::new(
            move |parent_session_id: String,
                  plan_content: String,
                  step_selection: Option<Vec<usize>>|
                  -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<String, String>> + Send>,
            > {
                let sm = Arc::clone(&sm);
                let cm = Arc::clone(&cm);
                Box::pin(async move {
                    let agent_id = sm.get_chat_id(&parent_session_id).await.unwrap_or_default();
                    let config = {
                        let agents = cm.agents.read().unwrap();
                        agents.get(&agent_id).cloned()
                    }
                    .ok_or_else(|| format!("agent config not found for agent_id={}", agent_id))?;
                    let depth = sm.get_session_depth(&parent_session_id).await.unwrap_or(0);
                    let task = format!(
                        "Execute plan (new session). Step selection: {:?}",
                        step_selection
                    );
                    let prompt_prefix = format!(
                        "## Plan Content (auto-injected for new session execution)\n\n{}",
                        plan_content
                    );
                    let max_spawn_depth = sm
                        .get_effective_max_spawn_depth(&parent_session_id)
                        .await
                        .unwrap_or(3);
                    use closeclaw_gateway::session_manager::{ChildSessionConfig, SpawnMode};
                    let child_config = ChildSessionConfig {
                        config,
                        parent_session_id,
                        depth: depth + 1,
                        task,
                        light_context: false,
                        workspace: None,
                        mode: SpawnMode::Run,
                        fork: false,
                        allowed_tools: None,
                        model_override: None,
                        parent_subagents_model: None,
                        max_spawn_depth,
                        spawn_timeout: None,
                        label: Some("plan-execution".to_string()),
                        prompt_template_prefix: Some(prompt_prefix),
                        timeout_warning_secs: None,
                        timeout_notify_interval_ratio: None,
                    };
                    let child_id = sm.create_child_session_with_config(child_config).await?;
                    Ok(child_id)
                })
            },
        )
    }

    /// Phase 5: Background services — ArchiveSweeper, DreamingScheduler, registry population.
    async fn init_phase_5_background(
        deps: Phase5Deps<'_>,
        data_dir: &std::path::Path,
        session_config_provider: Arc<dyn closeclaw_config::session::SessionConfigProvider>,
    ) -> anyhow::Result<(
        watch::Sender<()>,
        watch::Sender<()>,
        watch::Sender<()>,
        config_watcher::ConfigWatcherHandle,
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<()>,
        Arc<SpawnController>,
        Arc<dyn closeclaw_common::SystemPromptBuilder>,
        tokio::sync::mpsc::Receiver<String>,
    )> {
        let Phase5Deps {
            config_manager,
            agent_registry,
            skill_registry,
            builtin_skill_registry,
            tool_registry,
            session_manager,
            permission_engine,
            approval_flow,
            gateway,
            slash_registry,
            shared_cache,
        } = deps;
        let (sweeper_tx, sweeper_rx) = watch::channel(());
        let (announce_sweeper_tx, announce_sweeper_rx) = watch::channel(());
        let (dreaming_tx, dreaming_rx) = watch::channel(());
        let (sweeper_handle, announce_sweeper_handle, dreaming_handle) =
            Self::spawn_background_services(
                config_manager,
                session_manager,
                data_dir,
                ServiceShutdownReceivers {
                    sweeper: sweeper_rx,
                    announce_sweeper: announce_sweeper_rx,
                    dreaming: dreaming_rx,
                },
                session_config_provider,
            );
        // Create SpawnController as an independent component (depends on AgentRegistry).
        let spawn_controller = Arc::new(closeclaw_gateway::SpawnController::new(
            Arc::clone(agent_registry),
            Arc::clone(config_manager),
            Arc::clone(session_manager),
            Arc::clone(permission_engine),
        ));
        let config_subdir = PathBuf::from(data_dir).join("config");
        let late_bound_session_manager =
            Arc::new(closeclaw_session::tools::LateBoundSessionManagerOps::new());
        let builtin_skill_listing = Arc::clone(builtin_skill_registry);
        // Create the restart signal channel.  The sender is captured
        // by the DaemonReloadCallback (via config_watcher) to signal
        // restart-class config changes; the receiver is consumed by
        // the daemon main loop to call request_gateway_restart().
        let (restart_tx, restart_rx) = tokio::sync::mpsc::channel(8);
        let ctx = registries::RegistryContext {
            config_manager,
            agent_registry,
            skill_registry,
            builtin_registry: builtin_skill_registry,
            tool_registry,
            session_manager,
            permission_engine,
            spawn_controller: Arc::clone(&spawn_controller),
            approval_flow,
            late_bound_session_manager: late_bound_session_manager.clone(),
            config_subdir: &config_subdir,
            data_dir,
            gateway,
            restart_tx: Some(restart_tx),
        };
        let config_watcher = registries::populate_registries(&ctx).await?;

        // Create SystemPromptBuilderAdapter and inject into SessionManager.
        // This bridges the SystemPromptBuilder trait (used by ConversationSession
        // for static-layer prompt construction) to the Provider-driven pipeline.
        //
        // AgentRegistry uses DashMap internally (interior mutability), but the
        // adapter API requires Arc<tokio::sync::RwLock<AgentRegistry>> — snapshot
        // the current configs into a new tokio RwLock-wrapped registry.
        let adapter_registry = {
            let new_reg = closeclaw_agent::registry::AgentRegistry::new();
            let configs: Vec<_> = agent_registry.iter().map(|e| e.value().clone()).collect();
            new_reg.populate(configs);
            Arc::new(tokio::sync::RwLock::new(new_reg))
        };
        let skill_provider: Arc<dyn closeclaw_common::SkillListingProvider> =
            Arc::new(crate::bridge::SkillListingProviderWrapper::new(
                skill_registry.clone(),
                Arc::clone(&builtin_skill_listing),
            ));
        // Build Provider list from domain crates (tools, skills, memory).
        // BootstrapFragmentProvider remains in system_prompt (its own crate's provider).
        let mut providers: Vec<Arc<dyn closeclaw_common::PromptFragmentProvider>> = vec![
            Arc::new(closeclaw_system_prompt::BootstrapFragmentProvider::new()),
            Arc::new(closeclaw_skills::SkillsFragmentProvider::new(
                skill_provider,
            )),
            Arc::new(closeclaw_memory::MemoryFragmentProvider::new()),
            Arc::new(closeclaw_tools::ToolsFragmentProvider::new(
                Arc::clone(tool_registry),
                Some(Arc::clone(agent_registry) as Arc<dyn closeclaw_common::AgentToolsConfigQuery>),
                None,
            )),
        ];
        providers.sort_by_key(|p| p.priority());
        let prompt_builder_adapter = Arc::new(
            closeclaw_system_prompt::adapter::SystemPromptBuilderAdapter::new_with_providers(
                adapter_registry,
                data_dir.to_path_buf(),
                Arc::clone(shared_cache),
                providers,
            ),
        ) as Arc<dyn closeclaw_common::SystemPromptBuilder>;
        session_manager
            .set_system_prompt_builder(Arc::clone(&prompt_builder_adapter))
            .await;
        info!("SystemPromptBuilder adapter injected into SessionManager");

        // Register SkillSlashHandler for all user-invocable skills.
        // Must happen after populate_registries so DiskSkillRegistry is loaded.
        {
            use closeclaw_slash::skill_handler::SkillSlashHandler;
            let disk_reg = {
                let guard = skill_registry.read().unwrap();
                guard.as_ref().map(|dr| Arc::new(dr.clone()))
            };
            if let Some(disk_reg) = disk_reg {
                let skill_handler = Arc::new(SkillSlashHandler::new(
                    disk_reg,
                    Arc::clone(builtin_skill_registry),
                ));
                for name in skill_handler.invocable_names().await {
                    slash_registry.register_named(
                        &name,
                        Arc::clone(&skill_handler) as Arc<dyn closeclaw_slash::SlashHandler>,
                    );
                }
                let count = slash_registry.all_commands().len();
                info!(count = count, "slash registry fully populated");
            }
        }
        // Inject the real SessionManager into the late-bound proxy so
        // session tools can delegate to it (layer 4 after layer 3).
        if late_bound_session_manager
            .set(Arc::clone(session_manager)
                as Arc<dyn closeclaw_session::tools::SessionManagerOps>)
            .is_err()
        {
            panic!("late_bound_session_manager should not be set twice");
        }
        session_manager
            .set_tool_registry(
                Arc::clone(tool_registry) as Arc<dyn closeclaw_common::ToolRegistryQuery>
            )
            .await;
        session_manager
            .set_skill_registry(Arc::new(crate::bridge::SkillRegistryWrapper(
                skill_registry.clone(),
            ))
                as Arc<dyn closeclaw_common::SkillRegistryQuery>)
            .await;
        // Inject skill listing provider so resolve() can pass it to every
        // new ConversationSession for per-turn skill attachment injection.
        session_manager
            .set_skill_listing_provider(Arc::new(crate::bridge::SkillListingProviderWrapper::new(
                skill_registry.clone(),
                Arc::clone(&builtin_skill_listing),
            ))
                as Arc<dyn closeclaw_common::SkillListingProvider>)
            .await;
        // Inject static-layer cache invalidation callback so /system clear
        // can invalidate section caches without gateway depending on
        // closeclaw-system-prompt directly.
        session_manager
            .set_cache_invalidator(Arc::new({
                let shared_cache = Arc::clone(shared_cache);
                move || {
                    shared_cache.write().unwrap().invalidate_all();
                }
            }))
            .await;
        // Inject dynamic prompt builder so resolve() and
        // force_new_for_channel() can pass it to every new
        // ConversationSession for per-request dynamic-layer injection.
        session_manager
            .set_dynamic_prompt_builder(Arc::new(
                closeclaw_system_prompt::SystemPromptDynamicBuilder,
            ))
            .await;
        Ok((
            sweeper_tx,
            announce_sweeper_tx,
            dreaming_tx,
            config_watcher,
            sweeper_handle,
            announce_sweeper_handle,
            dreaming_handle,
            spawn_controller,
            prompt_builder_adapter,
            restart_rx,
        ))
    }

    /// Spawn ArchiveSweeper and DreamingScheduler.
    ///
    /// PlanArchiveSweeper is spawned separately in `init_phase_2_registries`
    /// as a Layer 2 component (depends on ConfigManager).
    fn spawn_background_services(
        config_manager: &Arc<ConfigManager>,
        session_manager: &Arc<SessionManager>,
        data_dir: &std::path::Path,
        shutdown_receivers: ServiceShutdownReceivers,
        session_config_provider: Arc<dyn SessionConfigProvider>,
    ) -> (
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let ServiceShutdownReceivers {
            sweeper: sweeper_rx,
            announce_sweeper: announce_sweeper_rx,
            dreaming: dreaming_rx,
        } = shutdown_receivers;
        let dreaming_config_provider = Arc::clone(&session_config_provider);
        let storage: Arc<dyn PersistenceService> =
            Arc::new(SqliteStorage::new(data_dir).expect("SqliteStorage already initialized"))
                as Arc<dyn PersistenceService>;
        // Create mining notification channel: sweeper + sub-agent → scheduler
        let (mining_notify_tx, mining_notify_rx) = tokio::sync::mpsc::channel(32);
        session_manager.set_mining_notify_tx(mining_notify_tx.clone());
        let sweeper = Arc::new(
            ArchiveSweeper::new(Arc::clone(&storage), session_config_provider.clone())
                .with_mining_notify_tx(mining_notify_tx)
                .with_active_query(Arc::clone(session_manager)
                    as Arc<dyn closeclaw_gateway::sweeper::ActiveSessionQuery>),
        );
        let sweeper_for_task = Arc::clone(&sweeper);
        let sweeper_handle = tokio::spawn(async move {
            sweeper_for_task.run(sweeper_rx).await;
        });
        info!("ArchiveSweeper spawned");
        // Spawn AnnounceSweeper for spawn silent-failure protection.
        let announce_sweeper =
            closeclaw_session::run_health::AnnounceSweeper::new(Arc::clone(session_manager)
                as Arc<dyn closeclaw_session::run_health::AnnounceSweepTarget>);
        let announce_sweeper_handle = tokio::spawn(async move {
            announce_sweeper.run(announce_sweeper_rx).await;
        });
        info!("AnnounceSweeper spawned");
        // Spawn periodic consistency check (low-priority, non-blocking).
        {
            let check_interval_secs = session_config_provider.consistency_check_interval_secs();
            let check_interval = std::time::Duration::from_secs(check_interval_secs);
            session_manager.spawn_periodic_consistency_check(check_interval);
        }
        // Load memory config from ConfigManager (replaces hardcoded defaults).
        let memory_config = config_manager
            .section(closeclaw_config::ConfigSection::Memory)
            .and_then(|v| {
                let content = serde_json::to_string(&v).ok()?;
                closeclaw_config::providers::MemoryConfigData::from_json_str(&content).ok()
            })
            .unwrap_or_default();
        let db_path = memory_config
            .config
            .storage
            .db_path
            .as_deref()
            .unwrap_or("memory/memory.db");
        let md_path = memory_config
            .config
            .storage
            .memory_md_path
            .as_deref()
            .unwrap_or("memory/MEMORY.md");
        let dreaming_pipeline = Arc::new(
            DreamingPipeline::with_config(memory_config.config.dreaming.clone())
                .with_memory_md_path(md_path),
        );
        let memory_miner = Arc::new(MemoryMiner::new(
            closeclaw_memory::miner::MinerConfig::from_memory_config(&memory_config.config),
            Box::new(noop_miner_llm::NoopMinerLlmCaller),
            Box::new(noop_miner_llm::NoopMinerLlmCaller),
            data_dir.join(db_path),
            data_dir.join(md_path).to_string_lossy().into_owned(),
        ));
        let mut dreaming_scheduler = crate::dreaming_scheduler::DreamingScheduler::new(
            storage,
            dreaming_config_provider,
            dreaming_pipeline,
            memory_miner,
            Arc::clone(config_manager),
        )
        .with_schedule(Some(
            memory_config
                .config
                .dreaming
                .schedule
                .clone()
                .unwrap_or_else(closeclaw_config::agents::default_dreaming_schedule),
        ))
        .with_mining_notify_rx(mining_notify_rx);
        let dreaming_handle = tokio::spawn(async move {
            dreaming_scheduler.run(dreaming_rx).await;
        });
        info!("DreamingScheduler spawned");
        (sweeper_handle, announce_sweeper_handle, dreaming_handle)
    }

    /// Phase 6: Admin RPC Server — depends on Gateway (Layer 5).
    async fn init_phase_6_admin_rpc(
        agent_registry: &Arc<closeclaw_agent::registry::AgentRegistry>,
        skill_registry: &Arc<RwLock<Option<DiskSkillRegistry>>>,
        config_manager: &Arc<ConfigManager>,
        config_dir: &str,
        admin_restart_tx: tokio::sync::mpsc::Sender<bool>,
    ) -> (tokio::task::JoinHandle<()>, PathBuf) {
        let admin_sock_path = admin_socket_path(Path::new(config_dir));
        let admin_context = AdminContext {
            agent_registry: Arc::clone(agent_registry),
            skill_registry: skill_registry.clone(),
            config_manager: Arc::clone(config_manager),
            config_dir: PathBuf::from(config_dir),
            restart_tx: Some(admin_restart_tx),
        };
        let admin_server = AdminServer::new(&admin_sock_path, admin_context);
        let admin_handle = tokio::spawn(async move {
            if let Err(e) = admin_server.serve().await {
                tracing::error!(error = %e, "admin RPC server failed");
            }
        });
        info!("admin RPC server started on {}", admin_sock_path.display());
        (admin_handle, admin_sock_path)
    }

    /// Phase 6: Chat RPC Server — depends on Gateway (Layer 5).
    async fn init_phase_6_chat_rpc(
        gateway: &Arc<closeclaw_gateway::Gateway>,
        config_dir: &str,
    ) -> (tokio::task::JoinHandle<()>, PathBuf) {
        use crate::chat_rpc::{chat_socket_path, ChatContext, ChatRpcServer, RpcTerminalPlugin};
        let sock_path = chat_socket_path(Path::new(config_dir));
        let rpc_plugin = Arc::new(RpcTerminalPlugin::new());
        gateway
            .register_plugin(rpc_plugin.clone() as Arc<dyn closeclaw_common::IMPlugin>)
            .await;
        let context = ChatContext {
            gateway: Arc::clone(gateway),
            rpc_plugin,
        };
        let chat_server = ChatRpcServer::new(&sock_path, context);
        let chat_handle = tokio::spawn(async move {
            if let Err(e) = chat_server.serve().await {
                tracing::error!(error = %e, "chat RPC server failed");
            }
        });
        info!("chat RPC server started on {}", sock_path.display());
        (chat_handle, sock_path)
    }
}
#[cfg(test)]
mod daemon_shutdown_tests;
#[cfg(test)]
mod dreaming_scheduler_tests;
#[cfg(test)]
mod gateway_restart_checkpoint_tests;
#[cfg(test)]
mod lifecycle_abort_tests;
#[cfg(test)]
mod lifecycle_assembly_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod session_config_provider_tests;
#[cfg(test)]
mod shutdown_alignment_tests;
#[cfg(test)]
mod shutdown_tests;
#[cfg(test)]
#[path = "spawn_controller_crate_reexport_tests.rs"]
mod spawn_controller_crate_reexport_tests;
#[cfg(test)]
mod step14_comprehensive_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod unit_tests;
