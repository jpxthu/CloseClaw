//! Daemon - CloseClaw background service
//!
//! Orchestrates all components: Gateway, AgentRegistry, PermissionEngine.
//! Handles graceful shutdown via ShutdownCoordinator.
pub mod bridge;
pub mod config_reload;
pub mod config_watcher;
mod daemon_struct;
pub mod dreaming_scheduler;
pub mod lifecycle;
pub mod registries;
pub mod shutdown;
pub mod skill_reload;
pub mod startup;
pub mod trait_adapters;
use crate::startup::{all_component_entries, topo_sort_layers, StartupError};
use closeclaw_cli::admin::{admin_socket_path, AdminContext, AdminServer};
use closeclaw_common::NoopMetricsEmitter;
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
use closeclaw_common::SessionLookup;
use closeclaw_gateway::sweeper::ArchiveSweeper;
/// Re-export `SpawnController` from gateway so consumers can access it
/// via `closeclaw_daemon::SpawnController`, aligning with the design doc
/// which places SpawnController at the daemon layer.
pub use closeclaw_gateway::SpawnController;
use closeclaw_gateway::{Gateway, GatewayConfig, SessionManager};
use closeclaw_memory::dreaming::DreamingPipeline;
use closeclaw_memory::miner::MemoryMiner;
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::{PermissionEngine, RuleSet};
use closeclaw_session::checkpoint_manager::CheckpointManager;
use closeclaw_session::persistence::PersistenceService;
use closeclaw_session::storage::SqliteStorage;
use closeclaw_skills::builtin::builtin_skills;
use closeclaw_skills::{BuiltinSkillRegistry, DiskSkillRegistry, SkillWatcherHandle};
use closeclaw_system_prompt::sections::SectionCache;
use closeclaw_tools::ToolRegistry;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::sync::watch;
use tracing::info;
mod noop_miner_llm;
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
        use crate::startup::ComponentId::*;
        let expected: Vec<_> = [
            vec![ConfigManager, Storage],
            vec![
                AgentRegistry,
                ConfigHotReload,
                PermissionEngine,
                RenderersPlugins,
                SessionConfigProvider,
                SkillsRegistry,
            ],
            vec![
                AnnounceSweeper,
                ApprovalFlow,
                ArchiveSweeper,
                DreamingScheduler,
                IMAdapters,
                SkillWatcher,
                ToolsRegistry,
            ],
            vec![SessionManager, SpawnController, SystemPromptBuilder],
            vec![Gateway],
            vec![AdminRpcServer],
        ]
        .into_iter()
        .collect();
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

    /// Phase 2: Registries — AgentRegistry, SkillsRegistry, ToolsRegistry.
    async fn init_phase_2_registries(
        config_dir: &str,
        config_manager: &ConfigManager,
    ) -> anyhow::Result<(
        Arc<closeclaw_agent::registry::AgentRegistry>,
        Arc<RwLock<Option<DiskSkillRegistry>>>,
        Arc<ToolRegistry>,
        Option<SkillWatcherHandle>,
        Arc<RwLock<SectionCache>>,
        Arc<dyn SessionConfigProvider>,
    )> {
        let agent_registry = Arc::new(closeclaw_agent::registry::AgentRegistry::new());
        info!("Agent registry initialized");
        let shared_cache = Arc::new(RwLock::new(SectionCache::new()));
        let extra_dirs = Self::resolve_extra_dirs(config_manager);
        let (skill_registry, skill_watcher) = skill_reload::init_skill_hot_reload(
            config_dir,
            None,
            Arc::clone(&shared_cache),
            extra_dirs,
        )
        .await?;
        let tool_registry = Arc::new(ToolRegistry::new());
        let session_config_provider = config_manager
            .session_config_provider()
            .unwrap_or_else(|| {
                tracing::warn!(
                    "session config provider not available after load, using defaults"
                );
                Arc::new(
                    closeclaw_config::session::JsonSessionConfigProvider::new("/dev/null")
                        .unwrap(),
                )
            });
        Ok((
            agent_registry,
            skill_registry,
            tool_registry,
            skill_watcher,
            shared_cache,
            session_config_provider,
        ))
    }

    /// Resolve extra skill directories from system config.
    ///
    /// Reads `skills.extraDirs` from `SystemConfigData` and expands
    /// `~` to the user's home directory. Non-existent paths are kept
    /// as-is; the loader layer handles skipping them per the design doc.
    fn resolve_extra_dirs(config_manager: &ConfigManager) -> Vec<PathBuf> {
        let Some(system_value) = config_manager.section(ConfigSection::System) else {
            return Vec::new();
        };
        let Ok(sys_cfg) = serde_json::from_value::<SystemConfigData>(system_value) else {
            return Vec::new();
        };
        let Some(skills_cfg) = sys_cfg.skills else {
            return Vec::new();
        };
        let home = dirs::home_dir();
        skills_cfg
            .extra_dirs
            .iter()
            .map(|dir| {
                if let Some(rest) = dir.strip_prefix("~/") {
                    home.as_ref()
                        .map(|h| h.join(rest))
                        .unwrap_or_else(|| PathBuf::from(dir))
                } else {
                    PathBuf::from(dir)
                }
            })
            .collect()
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

// --- Phase 4-5 initialization ---
impl Daemon {
    /// Phase 4: Wiring — ApprovalFlow.
    async fn init_phase_4_wiring(
        gateway: &Arc<Gateway>,
        session_manager: &Arc<SessionManager>,
        permission_engine: &Arc<tokio::sync::RwLock<PermissionEngine>>,
        config_manager: &Arc<closeclaw_config::ConfigManager>,
        config_dir: &str,
    ) -> (
        Arc<tokio::sync::Mutex<ApprovalFlow>>,
        Arc<BuiltinSkillRegistry>,
    ) {
        // Build the whitelist-updated callback: reads agent permissions.json,
        // constructs a RuleSet, and reloads the permission engine.
        //
        // The approval flow is created after this callback, so we use a
        // OnceLock to defer the reference. The callback updates both the
        // permission engine and the approval flow snapshot on hot-reload.
        let pe_clone = Arc::clone(permission_engine);
        let cfg_dir = std::path::PathBuf::from(config_dir);
        let af_ref = Arc::new(std::sync::OnceLock::<Arc<tokio::sync::Mutex<ApprovalFlow>>>::new());
        let af_ref_clone = Arc::clone(&af_ref);
        let whitelist_cb: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |agent_id: &str| {
            let path = cfg_dir
                .join("agents")
                .join(agent_id)
                .join("permissions.json");
            match std::fs::read_to_string(&path) {
                Ok(json) => {
                    match serde_json::from_str::<closeclaw_permission::RuleSet>(&json) {
                        Ok(ruleset) => {
                            // Best-effort: try_write avoids blocking the approval flow.
                            if let Ok(mut guard) = pe_clone.try_write() {
                                guard.reload_rules(ruleset.clone());
                                tracing::info!(
                                    agent = %agent_id,
                                    "whitelist rules reloaded after approval"
                                );
                            } else {
                                tracing::warn!(
                                    agent = %agent_id,
                                    "permission engine write lock contended, skipping hot-reload"
                                );
                            }
                            // Sync approval flow snapshot so subsequent snapshots
                            // reflect the updated rules.
                            if let Some(af) = af_ref_clone.get() {
                                if let Ok(mut af_guard) = af.try_lock() {
                                    af_guard.update_rules(ruleset);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                agent = %agent_id,
                                error = %e,
                                "failed to parse whitelist rules from permissions.json"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        agent = %agent_id,
                        error = %e,
                        "failed to read permissions.json for whitelist reload"
                    );
                }
            }
        });

        // Build the child-session creation callback for the new-session
        // execution path. The callback captures SessionManager and
        // ConfigManager to resolve agent config at call time.
        let sm_for_spawn = Arc::clone(session_manager);
        let cm_for_spawn = Arc::clone(config_manager);
        let create_child_fn: closeclaw_permission::approval_flow::CreateChildSessionFn = Arc::new(
            move |parent_session_id: String,
                  plan_content: String,
                  step_selection: Option<Vec<usize>>|
                  -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<String, String>> + Send>,
            > {
                let sm = Arc::clone(&sm_for_spawn);
                let cm = Arc::clone(&cm_for_spawn);
                Box::pin(async move {
                    // Resolve the parent session's agent_id.
                    let agent_id = sm.get_chat_id(&parent_session_id).await.unwrap_or_default();

                    // Resolve agent config from ConfigManager.
                    let config = {
                        let agents = cm.agents.read().unwrap();
                        agents.get(&agent_id).cloned()
                    }
                    .ok_or_else(|| format!("agent config not found for agent_id={}", agent_id))?;

                    // Determine parent session depth.
                    let depth = sm.get_session_depth(&parent_session_id).await.unwrap_or(0);

                    // Build task description and inject plan content as initial context.
                    let task = format!(
                        "Execute plan (new session). Step selection: {:?}",
                        step_selection
                    );
                    // Inject the full plan file content into the system prompt
                    // so the new session has complete plan context from the start.
                    let prompt_prefix = format!(
                        "## Plan Content (auto-injected for new session execution)\n\n{}",
                        plan_content
                    );

                    // Determine max_spawn_depth from parent session.
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
                    };
                    let child_id = sm.create_child_session_with_config(child_config).await?;

                    // The child session is created in Run mode by default.
                    // The approval flow's handle_new_session_path will set
                    // Auto Mode and plan state after this callback returns.
                    Ok(child_id)
                })
            },
        );

        let mut af = ApprovalFlow::new(
            Arc::clone(session_manager) as Arc<dyn SessionLookup>,
            Arc::new(|_| {}),
            whitelist_cb,
            tokio::runtime::Handle::current(),
            HeartbeatApprovalMode::default(),
            std::path::PathBuf::from(config_dir),
            RuleSet::default(),
        );
        af.set_create_child_session_fn(create_child_fn);
        let approval_flow = Arc::new(tokio::sync::Mutex::new(af));
        // Wire the approval flow into the whitelist callback so hot-reload
        // updates the snapshot too (see OnceLock in the callback above).
        let _ = af_ref.set(Arc::clone(&approval_flow));

        // Sync approval flow snapshot with actual loaded rules.
        // Without this, the approval flow holds `RuleSet::default()` and
        // all snapshots would evaluate against empty rules.
        {
            let pe_guard = permission_engine.read().await;
            let engine_rules = pe_guard.rules().clone();
            drop(pe_guard);
            approval_flow.lock().await.update_rules(engine_rules);
        }

        gateway.set_approval_flow(Arc::clone(&approval_flow)).await;
        let builtin_skills = builtin_skills();
        let builtin_skill_registry =
            Arc::new(BuiltinSkillRegistry::from_skills(builtin_skills).await);
        let builtin_count = builtin_skill_registry.list().await.len();
        info!(
            count = builtin_count,
            "builtin skills registered in BuiltinSkillRegistry"
        );
        (approval_flow, builtin_skill_registry)
    }

    /// Phase 5: Background services — ArchiveSweeper, DreamingScheduler, registry population.
    async fn init_phase_5_background(
        deps: Phase5Deps<'_>,
        data_dir: &std::path::Path,
    ) -> anyhow::Result<(
        watch::Sender<()>,
        watch::Sender<()>,
        watch::Sender<()>,
        watch::Sender<()>,
        Option<config_watcher::ConfigWatcherHandle>,
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<()>,
        Arc<SpawnController>,
        Arc<dyn closeclaw_common::SystemPromptBuilder>,
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
        let (plan_archive_tx, plan_archive_rx) = watch::channel(());
        let (sweeper_handle, announce_sweeper_handle, dreaming_handle, plan_archive_handle) =
            Self::spawn_background_services(
                config_manager,
                session_manager,
                data_dir,
                sweeper_rx,
                announce_sweeper_rx,
                dreaming_rx,
                plan_archive_rx,
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
            gateway,
        };
        let config_watcher = registries::populate_registries(&ctx).await;

        // Create SystemPromptBuilderAdapter and inject into SessionManager.
        // This bridges the SystemPromptBuilder trait (used by ConversationSession
        // for static-layer prompt construction) to the Provider-driven pipeline.
        //
        // AgentRegistry uses DashMap internally (interior mutability), but the
        // adapter API requires Arc<std::sync::RwLock<AgentRegistry>> — snapshot
        // the current configs into a new RwLock-wrapped registry.
        let adapter_registry = {
            let new_reg = closeclaw_agent::registry::AgentRegistry::new();
            let configs: Vec<_> = agent_registry.iter().map(|e| e.value().clone()).collect();
            new_reg.populate(configs);
            Arc::new(std::sync::RwLock::new(new_reg))
        };
        let skill_provider: Arc<dyn closeclaw_common::SkillListingProvider> =
            Arc::new(crate::bridge::SkillListingProviderWrapper::new(
                skill_registry.clone(),
                Arc::clone(&builtin_skill_listing),
            ));
        let prompt_builder_adapter = Arc::new(
            closeclaw_system_prompt::adapter::SystemPromptBuilderAdapter::new_with_cache(
                Arc::clone(tool_registry),
                adapter_registry,
                data_dir.to_path_buf(),
                Arc::clone(shared_cache),
                Some(skill_provider),
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
            plan_archive_tx,
            config_watcher,
            sweeper_handle,
            announce_sweeper_handle,
            dreaming_handle,
            plan_archive_handle,
            spawn_controller,
            prompt_builder_adapter,
        ))
    }

    /// Spawn ArchiveSweeper, DreamingScheduler, and PlanArchiveTask.
    fn spawn_background_services(
        config_manager: &Arc<ConfigManager>,
        session_manager: &Arc<SessionManager>,
        data_dir: &std::path::Path,
        sweeper_rx: watch::Receiver<()>,
        announce_sweeper_rx: watch::Receiver<()>,
        dreaming_rx: watch::Receiver<()>,
        plan_archive_rx: watch::Receiver<()>,
    ) -> (
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let session_config_provider =
            config_manager.session_config_provider().unwrap_or_else(|| {
                tracing::warn!("session config provider not available, using defaults");
                Arc::new(
                    closeclaw_config::session::JsonSessionConfigProvider::new("/dev/null").unwrap(),
                )
            });
        let dreaming_config_provider = Arc::clone(&session_config_provider);
        let storage: Arc<dyn PersistenceService> =
            Arc::new(SqliteStorage::new(data_dir).expect("SqliteStorage already initialized"))
                as Arc<dyn PersistenceService>;
        // Create mining notification channel: sweeper + sub-agent → scheduler
        let (mining_notify_tx, mining_notify_rx) = tokio::sync::mpsc::channel(32);
        session_manager.set_mining_notify_tx(mining_notify_tx.clone());

        let sweeper = Arc::new(
            ArchiveSweeper::new(Arc::clone(&storage), session_config_provider)
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
            let check_interval_secs = config_manager
                .session_config_provider()
                .map(|p| p.consistency_check_interval_secs())
                .unwrap_or(closeclaw_config::session::DEFAULT_CONSISTENCY_CHECK_INTERVAL_SECS);
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
        let dreaming_pipeline = Arc::new(DreamingPipeline::with_config(
            memory_config.config.dreaming.clone(),
        ));
        let memory_miner = Arc::new(MemoryMiner::new(
            closeclaw_memory::miner::MinerConfig::from_memory_config(&memory_config.config),
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
        // Spawn PlanArchiveTask for periodic plan file archival.
        let plan_archive_threshold_days = config_manager
            .section(closeclaw_config::ConfigSection::System)
            .and_then(|v| {
                serde_json::from_value::<closeclaw_config::providers::SystemConfigData>(v).ok()
            })
            .and_then(|sys| sys.plan_archive)
            .map(|p| p.threshold_days)
            .unwrap_or(closeclaw_session::plan_archive::DEFAULT_THRESHOLD_DAYS);
        let plan_archive_task = closeclaw_session::background::PlanArchiveTask::new(
            data_dir.to_path_buf(),
            plan_archive_threshold_days,
        );
        let plan_archive_handle = tokio::spawn(async move {
            plan_archive_task.run(plan_archive_rx).await;
        });
        info!("PlanArchiveTask spawned");
        (
            sweeper_handle,
            announce_sweeper_handle,
            dreaming_handle,
            plan_archive_handle,
        )
    }

    /// Phase 6: Admin RPC Server — depends on Gateway (Layer 5).
    async fn init_phase_6_admin_rpc(
        agent_registry: &Arc<closeclaw_agent::registry::AgentRegistry>,
        skill_registry: &Arc<RwLock<Option<DiskSkillRegistry>>>,
        config_manager: &Arc<ConfigManager>,
        config_dir: &str,
    ) -> (tokio::task::JoinHandle<()>, PathBuf) {
        let admin_sock_path = admin_socket_path(Path::new(config_dir));
        let admin_context = AdminContext {
            agent_registry: Arc::clone(agent_registry),
            skill_registry: skill_registry.clone(),
            config_manager: Arc::clone(config_manager),
            config_dir: PathBuf::from(config_dir),
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
}

#[cfg(test)]
mod dreaming_scheduler_tests;
#[cfg(test)]
mod lifecycle_abort_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod shutdown_alignment_tests;
#[cfg(test)]
mod shutdown_tests;
#[cfg(test)]
#[path = "spawn_controller_crate_reexport_tests.rs"]
mod spawn_controller_crate_reexport_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod unit_tests;
