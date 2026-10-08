//! Phase 4-6 daemon wiring: approval flow, background services, RPC servers.
//!
//! Extracted from `mod.rs` to keep source files within the CONTRIBUTING.md
//! limits (`mod.rs` only holds `pub use` / `pub mod` re-exports).

use super::{daemon_struct::Phase5Deps, phase_init::ServiceShutdownReceivers, Daemon};
use crate::{
    bridge::{SkillListingProviderWrapper, SkillRegistryWrapper},
    chat_rpc::{
        chat_socket_path, spawn_chat_rpc_server, spawn_turn_completion_consumer, ChatRpcInit,
    },
    config_watcher,
    dreaming_scheduler::DreamingScheduler,
    noop_miner_llm, registries,
    skill_access_adapter::{builtin_skill_access, disk_skill_access},
    trait_adapters::{spawn_target_agent_config, ConfigSpawnBudgetLookup},
    workflow_port_adapter::workflow_definition_validator,
};
use closeclaw_agent::registry::AgentRegistry;
use closeclaw_cli::admin::{admin_socket_path, AdminContext, AdminServer};
use closeclaw_common::processor::ContentBlock;
use closeclaw_common::AuditLogger;
use closeclaw_common::TaskManager;
use closeclaw_common::{
    AgentLookup, AgentToolsConfigQuery, PermissionChecker, PromptFragmentProvider, SessionLookup,
    SkillListingProvider, SkillRegistryQuery, SystemPromptBuilder, ToolRegistryQuery,
};
use closeclaw_config::providers::MemoryConfigData;
use closeclaw_config::session::SessionConfigProvider;
use closeclaw_config::ConfigManager;
use closeclaw_config::{agents::default_dreaming_schedule, ConfigSection};
use closeclaw_debug_log::DebugLog;
use closeclaw_gateway::session_manager::spawn_adapter::GatewayPermissionChecker;
use closeclaw_gateway::session_manager::{ChildSessionConfig, SpawnMode};
use closeclaw_gateway::sweeper::ActiveSessionQuery;
use closeclaw_gateway::SpawnController;
use closeclaw_gateway::{sweeper::ArchiveSweeper, Gateway, SessionManager};
use closeclaw_memory::dreaming::DreamingPipeline;
use closeclaw_memory::miner::MemoryMiner;
use closeclaw_memory::MemoryFragmentProvider;
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::{PermissionEngine, RuleSet};
use closeclaw_session::bootstrap::loader::{bootstrap_file_list, load_bootstrap_files};
use closeclaw_session::run_health::{AnnounceSweepTarget, AnnounceSweeper};
use closeclaw_session::spawn::controller::{SpawnBudgetLookup, SpawnContext};
use closeclaw_session::tools::{LateBoundSessionManagerOps, SessionManagerOps};
use closeclaw_session::{persistence::PersistenceService, storage::SqliteStorage};
use closeclaw_skills::builtin::builtin_skills;
use closeclaw_skills::{BuiltinSkillRegistry, DiskSkillRegistry, SkillsFragmentProvider};
use closeclaw_slash::{skill_handler::SkillSlashHandler, SlashHandler};
use closeclaw_system_prompt::adapter::SystemPromptBuilderAdapter;
use closeclaw_system_prompt::{BootstrapFragmentProvider, SystemPromptDynamicBuilder};
use closeclaw_tasks::BackgroundTaskManager;
use closeclaw_tools::builtin::CreateChildSessionFn;
use closeclaw_tools::ToolsFragmentProvider;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::{mpsc, watch, Mutex};
use tokio::task::JoinHandle;
use tracing::{info, warn};

// --- Phase 4-5 initialization ---
impl Daemon {
    /// Phase 4: Wiring — ApprovalFlow.
    /// ApprovalFlow and BuiltinSkillRegistry initialized in parallel.
    pub(crate) async fn init_phase_4_wiring(
        gateway: &Arc<Gateway>,
        session_manager: &Arc<SessionManager>,
        permission_engine: &Arc<tokio::sync::RwLock<PermissionEngine>>,
        config_dir: &str,
        audit_logger: Option<Arc<dyn AuditLogger>>,
    ) -> (Arc<Mutex<ApprovalFlow>>, Arc<BuiltinSkillRegistry>) {
        // Build the whitelist-updated callback: invalidate the agent's
        // cached rules so the next evaluate() lazily re-reads from disk.
        let pe_clone = Arc::clone(permission_engine);
        let whitelist_cb: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |agent_id: &str| {
            if let Ok(guard) = pe_clone.try_write() {
                guard.invalidate_agent_rules(agent_id);
                tracing::info!(
                    agent = %agent_id,
                    "agent rule cache invalidated after whitelist approval"
                );
            } else {
                warn!(
                    agent = %agent_id,
                    "permission engine write lock contended, skipping cache invalidation"
                );
            }
        });
        let mut af = ApprovalFlow::new(
            Arc::clone(session_manager) as Arc<dyn SessionLookup>,
            Arc::new(|_| {}),
            whitelist_cb,
            tokio::runtime::Handle::current(),
            HeartbeatApprovalMode::default(),
            PathBuf::from(config_dir),
            RuleSet::default(),
        );
        if let Some(logger) = audit_logger {
            af = af.with_audit_logger(logger);
        }
        let approval_flow = Arc::new(Mutex::new(af));
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
            let skills = builtin_skills(workflow_definition_validator());
            let reg = Arc::new(BuiltinSkillRegistry::from_skills(skills).await);
            let count = reg.list().await.len();
            info!(count, "builtin skills registered in BuiltinSkillRegistry");
            reg
        };
        let ((), builtin_skill_registry) = tokio::join!(approval_fut, builtin_fut);
        (approval_flow, builtin_skill_registry)
    }
    /// Build the child-session creation callback for plan execution.
    pub(crate) fn build_create_child_fn(
        sm: Arc<SessionManager>,
        cm: Arc<ConfigManager>,
    ) -> CreateChildSessionFn {
        Arc::new(
            move |parent_session_id: String,
                  plan_content: String,
                  step_selection: Option<Vec<usize>>|
                  -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
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
                    // Map the full resolved config onto the session-owned
                    // narrow spawn-time view (pure data copy — the gateway
                    // boundary only carries creation-chain fields). Shared
                    // mapping with trait_adapters (single-point definition).
                    let target_config = spawn_target_agent_config(&config);
                    let child_config = ChildSessionConfig {
                        config: target_config,
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
    pub(crate) async fn init_phase_5_background(
        deps: Phase5Deps<'_>,
        data_dir: &Path,
        session_config_provider: Arc<dyn SessionConfigProvider>,
    ) -> anyhow::Result<(
        watch::Sender<()>,
        watch::Sender<()>,
        watch::Sender<()>,
        config_watcher::ConfigWatcherHandle,
        JoinHandle<()>,
        JoinHandle<()>,
        JoinHandle<()>,
        Arc<SpawnController>,
        Arc<dyn SystemPromptBuilder>,
        mpsc::Receiver<String>,
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
            confirm_flow,
            gateway,
            slash_registry,
            shared_cache,
        } = deps;
        let (sweeper_tx, sweeper_rx) = watch::channel(());
        let (announce_sweeper_tx, announce_sweeper_rx) = watch::channel(());
        let (dreaming_tx, dreaming_rx) = watch::channel(());
        let (sweeper_handle, announce_sweeper_handle, dreaming_handle, task_manager) =
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
                gateway.get_debug_log(),
            );
        session_manager.set_task_manager(task_manager).await;
        let spawn_controller = Arc::new({
            let pc: Arc<dyn PermissionChecker> = Arc::new(GatewayPermissionChecker::new(
                Arc::clone(session_manager),
                Arc::clone(config_manager),
                Arc::clone(permission_engine),
            ));
            let budget_lookup: Arc<dyn SpawnBudgetLookup> =
                Arc::new(ConfigSpawnBudgetLookup::new(Arc::clone(config_manager)));
            SpawnController::new(
                budget_lookup,
                Arc::clone(session_manager) as Arc<dyn SpawnContext>,
                pc,
            )
        });
        let config_subdir = PathBuf::from(data_dir).join("config");
        let late_bound_session_manager = Arc::new(LateBoundSessionManagerOps::new());
        let builtin_skill_listing = Arc::clone(builtin_skill_registry);
        // Create the restart signal channel. Sender captured by
        // DaemonReloadCallback; receiver consumed by daemon main loop.
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
            confirm_flow,
            late_bound_session_manager: late_bound_session_manager.clone(),
            config_subdir: &config_subdir,
            data_dir,
            gateway,
            restart_tx: Some(restart_tx),
        };
        let config_watcher = registries::populate_registries(&ctx).await?;
        // Create SystemPromptBuilderAdapter — bridges SystemPromptBuilder trait
        // to the Provider-driven pipeline.
        let agent_lookup: Arc<dyn AgentLookup> = {
            let new_reg = AgentRegistry::new();
            let configs: Vec<_> = agent_registry.iter().map(|e| e.value().clone()).collect();
            new_reg.populate(configs);
            Arc::new(new_reg)
        };
        let skill_provider: Arc<dyn SkillListingProvider> =
            Arc::new(SkillListingProviderWrapper::new(
                skill_registry.clone(),
                Arc::clone(&builtin_skill_listing),
            ));
        // Build Provider list from domain crates.
        let mut providers: Vec<Arc<dyn PromptFragmentProvider>> = vec![
            Arc::new(BootstrapFragmentProvider::new(
                bootstrap_file_list,
                |dir, mode| load_bootstrap_files(dir, mode).ok(),
            )),
            Arc::new(SkillsFragmentProvider::new(skill_provider)),
            Arc::new(MemoryFragmentProvider::new()),
            Arc::new(ToolsFragmentProvider::new(
                Arc::clone(tool_registry),
                Some(Arc::clone(agent_registry) as Arc<dyn AgentToolsConfigQuery>),
                None,
            )),
        ];
        providers.sort_by_key(|p| p.priority());
        let prompt_builder_adapter = Arc::new(SystemPromptBuilderAdapter::new_with_providers(
            agent_lookup,
            data_dir.to_path_buf(),
            Arc::clone(shared_cache),
            providers,
        )) as Arc<dyn SystemPromptBuilder>;
        session_manager
            .set_system_prompt_builder(Arc::clone(&prompt_builder_adapter))
            .await;
        info!("SystemPromptBuilder adapter injected into SessionManager");
        // Register SkillSlashHandler for all user-invocable skills.
        // Must happen after populate_registries so DiskSkillRegistry is loaded.
        {
            let disk_reg = {
                let guard = skill_registry.read().unwrap();
                guard.as_ref().map(|dr| Arc::new(dr.clone()))
            };
            if let Some(disk_reg) = disk_reg {
                let skill_handler = Arc::new(SkillSlashHandler::new(
                    disk_skill_access(disk_reg),
                    builtin_skill_access(Arc::clone(builtin_skill_registry)),
                ));
                for name in skill_handler.invocable_names().await {
                    slash_registry
                        .register_named(&name, Arc::clone(&skill_handler) as Arc<dyn SlashHandler>);
                }
                let count = slash_registry.all_commands().len();
                info!(count = count, "slash registry fully populated");
            }
        }
        // Inject real SessionManager into late-bound proxy (layer 4 after layer 3).
        if late_bound_session_manager
            .set(Arc::clone(session_manager) as Arc<dyn SessionManagerOps>)
            .is_err()
        {
            panic!("late_bound_session_manager should not be set twice");
        }
        session_manager
            .set_tool_registry(Arc::clone(tool_registry) as Arc<dyn ToolRegistryQuery>)
            .await;
        session_manager
            .set_skill_registry(Arc::new(SkillRegistryWrapper(skill_registry.clone()))
                as Arc<dyn SkillRegistryQuery>)
            .await;
        // Inject skill listing provider for per-turn skill attachment.
        session_manager
            .set_skill_listing_provider(Arc::new(SkillListingProviderWrapper::new(
                skill_registry.clone(),
                Arc::clone(&builtin_skill_listing),
            )) as Arc<dyn SkillListingProvider>)
            .await;
        // Inject the production workflow engine port (stateless adapter
        // over the workflow crate). Wired onto every created/restored
        // session and handed to child session creation; also used by
        // the recovery scan in phase_init.
        session_manager
            .set_workflow_port(crate::workflow_port_adapter::engine_workflow_port())
            .await;
        // Inject static-layer cache invalidation callback.
        session_manager
            .set_cache_invalidator(Arc::new({
                let shared_cache = Arc::clone(shared_cache);
                move || {
                    shared_cache.write().unwrap().invalidate_all();
                }
            }))
            .await;
        // Inject dynamic prompt builder for dynamic-layer injection.
        session_manager
            .set_dynamic_prompt_builder(Arc::new(SystemPromptDynamicBuilder))
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
    pub(crate) fn spawn_background_services(
        config_manager: &Arc<ConfigManager>,
        session_manager: &Arc<SessionManager>,
        data_dir: &Path,
        shutdown_receivers: ServiceShutdownReceivers,
        session_config_provider: Arc<dyn SessionConfigProvider>,
        debug_log: Option<DebugLog>,
    ) -> (
        JoinHandle<()>,
        JoinHandle<()>,
        JoinHandle<()>,
        Arc<dyn TaskManager>,
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
        let mut task_mgr = BackgroundTaskManager::new();
        if let Some(dl) = debug_log {
            task_mgr = task_mgr.with_debug_log(Arc::new(dl));
        }
        let task_manager: Arc<dyn TaskManager> = Arc::new(task_mgr);
        let sweeper = Arc::new(
            ArchiveSweeper::new(Arc::clone(&storage), session_config_provider.clone())
                .with_mining_notify_tx(mining_notify_tx)
                .with_active_query(Arc::clone(session_manager) as Arc<dyn ActiveSessionQuery>)
                .with_task_manager(Arc::clone(&task_manager)),
        );
        let sweeper_for_task = Arc::clone(&sweeper);
        let sweeper_handle = tokio::spawn(async move {
            sweeper_for_task.run(sweeper_rx).await;
        });
        info!("ArchiveSweeper spawned");
        // Spawn AnnounceSweeper for spawn silent-failure protection.
        let announce_sweeper =
            AnnounceSweeper::new(Arc::clone(session_manager) as Arc<dyn AnnounceSweepTarget>);
        let announce_sweeper_handle = tokio::spawn(async move {
            announce_sweeper.run(announce_sweeper_rx).await;
        });
        info!("AnnounceSweeper spawned");
        // Spawn periodic consistency check (low-priority, non-blocking).
        {
            let check_interval_secs = session_config_provider.consistency_check_interval_secs();
            let check_interval = Duration::from_secs(check_interval_secs);
            session_manager.spawn_periodic_consistency_check(check_interval);
        }
        // Load memory config from ConfigManager (replaces hardcoded defaults).
        let memory_config = config_manager
            .section(ConfigSection::Memory)
            .and_then(|v| {
                let content = serde_json::to_string(&v).ok()?;
                MemoryConfigData::from_json_str(&content).ok()
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
            DreamingPipeline::with_config(
                crate::memory_params_adapter::dreaming_params_from_config(
                    &memory_config.config.dreaming,
                ),
            )
            .with_memory_md_path(md_path),
        );
        let memory_miner = Arc::new(MemoryMiner::new(
            crate::memory_params_adapter::miner_config_from_memory(&memory_config.config),
            Box::new(noop_miner_llm::NoopMinerLlmCaller),
            Box::new(noop_miner_llm::NoopMinerLlmCaller),
            data_dir.join(db_path),
            data_dir.join(md_path).to_string_lossy().into_owned(),
        ));
        let mut dreaming_scheduler = DreamingScheduler::new(
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
                .unwrap_or_else(default_dreaming_schedule),
        ))
        .with_mining_notify_rx(mining_notify_rx);
        let dreaming_handle = tokio::spawn(async move {
            dreaming_scheduler.run(dreaming_rx).await;
        });
        info!("DreamingScheduler spawned");
        (
            sweeper_handle,
            announce_sweeper_handle,
            dreaming_handle,
            task_manager,
        )
    }
    /// Phase 6: Admin RPC Server — depends on Gateway (Layer 5).
    pub(crate) async fn init_phase_6_admin_rpc(
        agent_registry: &Arc<AgentRegistry>,
        skill_registry: &Arc<RwLock<Option<DiskSkillRegistry>>>,
        config_manager: &Arc<ConfigManager>,
        config_dir: &str,
        admin_restart_tx: mpsc::Sender<bool>,
    ) -> (JoinHandle<()>, PathBuf) {
        let admin_sock_path = admin_socket_path(Path::new(config_dir));
        let admin_context = AdminContext {
            agent_registry: Arc::clone(agent_registry),
            skill_registry: Some(Arc::new(SkillRegistryWrapper(skill_registry.clone()))
                as Arc<dyn SkillRegistryQuery>),
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
    ///
    /// Assembly lives in [`crate::chat_rpc::spawn_chat_rpc_server`]
    /// (shared with the gateway-restart path); this phase resolves the
    /// socket path, spawns the server, and wires the
    /// SessionMessageHandler output receiver into the shared
    /// turn-completion consumer (startup path's Step 1.11 wiring —
    /// single-point definition: the consumer's doc).
    pub(crate) async fn init_phase_6_chat_rpc(
        gateway: &Arc<Gateway>,
        config_dir: &str,
        output_rx: mpsc::Receiver<(String, Vec<ContentBlock>)>,
    ) -> ChatRpcInit {
        let sock_path = chat_socket_path(Path::new(config_dir));
        let (chat_handle, rpc_plugin) = spawn_chat_rpc_server(gateway, &sock_path).await;
        spawn_turn_completion_consumer(output_rx, rpc_plugin);
        (chat_handle, sock_path)
    }
}
