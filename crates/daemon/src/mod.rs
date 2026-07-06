//! Daemon - CloseClaw background service
//!
//! Orchestrates all components: Gateway, AgentRegistry, PermissionEngine.
//! Handles graceful shutdown via ShutdownCoordinator.
pub mod bridge;
pub mod config_reload;
pub mod config_watcher;
pub mod dreaming_scheduler;
pub mod registries;
pub mod shutdown;
pub mod skill_reload;
pub mod startup;
use crate::startup::{all_component_entries, topo_sort_layers, StartupError};
use closeclaw_cli::admin::{admin_socket_path, AdminContext, AdminServer};
use closeclaw_config::migration::migrate_if_needed;
use closeclaw_config::providers::ConfigProvider;
use closeclaw_config::ConfigManager;

/// Resolved startup plan: topo-sort layers plus validated phase components.
/// Each outer element is a layer/phase; each inner element is a [`ComponentId`].
type StartupPlan = (
    Vec<Vec<crate::startup::ComponentId>>,
    Vec<Vec<crate::startup::ComponentId>>,
);
use closeclaw_cli::terminal::TerminalPlugin;
use closeclaw_gateway::{DmScope, Gateway, GatewayConfig, SessionManager};
use closeclaw_processor_chain as processor_chain;
use closeclaw_slash::dispatcher::SlashDispatcher;
use closeclaw_slash::handlers::{ReasoningHandler, SystemHandler, WorkdirHandler};
use closeclaw_slash::registry::HandlerRegistry;
use closeclaw_slash::{
    ClearHandler, CompactHandler, ExecHandler, HelpHandler, NewSessionHandler, StatusHandler,
    StopHandler, VerboseHandler,
};

use closeclaw_common::SessionLookup;
use closeclaw_gateway::sweeper::ArchiveSweeper;
use closeclaw_memory::dreaming::DreamingPipeline;
use closeclaw_memory::miner::MemoryMiner;
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::{Defaults, PermissionEngine, RuleSet};
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::persistence::PersistenceService;
use closeclaw_session::persistence::ReasoningLevel;
use closeclaw_session::storage::SqliteStorage;
use closeclaw_skills::builtin::builtin_skills_with_engine_and_approval_flow;
use closeclaw_skills::{DiskSkillRegistry, SkillWatcherHandle};
use closeclaw_system_prompt::invalidate_all_sections;
use closeclaw_tools::ToolRegistry;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::sync::watch;
use tracing::info;

mod noop_miner_llm;

/// Parse an .env file into key-value pairs.
/// Lines starting with # are comments. Whitespace around keys and values is trimmed.
/// Returns only non-empty key-value pairs.
fn parse_env_file(path: &std::path::Path) -> std::io::Result<Vec<(String, String)>> {
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
/// Load key=value pairs from a .env file and set them as environment variables.
/// Lines starting with # are treated as comments and ignored.
fn load_env_file(path: &std::path::Path) -> std::io::Result<()> {
    for (key, value) in parse_env_file(path)? {
        std::env::set_var(&key, &value); // load_env_file: allowed exception per CONTRIBUTING.md
    }
    Ok(())
}
mod llm_init;
#[cfg(test)]
pub mod test_helpers;

/// Global daemon state
pub struct Daemon {
    pub gateway: Arc<Gateway>,
    pub agent_registry: Arc<closeclaw_agent::registry::AgentRegistry>,
    pub permission_engine: Arc<PermissionEngine>,
    pub shutdown: Arc<shutdown::ShutdownHandle>,
    /// Session manager for session lifecycle management
    pub session_manager: Arc<SessionManager>,
    /// SQLite storage for session persistence
    pub storage: Arc<SqliteStorage>,
    /// Shutdown sender for ArchiveSweeper
    pub sweeper_shutdown_tx: watch::Sender<()>,
    /// Shutdown sender for DreamingScheduler
    pub dreaming_scheduler_shutdown_tx: watch::Sender<()>,
    /// Shared skill registry, updated on hot reload
    pub skill_registry: Arc<RwLock<Option<DiskSkillRegistry>>>,
    /// Skill file watcher handle (RAII: stops on drop)
    _skill_watcher: Option<SkillWatcherHandle>,
    /// Config file watcher handle (RAII: stops on drop)
    _config_watcher: Option<config_watcher::ConfigWatcherHandle>,
    /// Daemon-level approval orchestrator
    pub approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
    /// Admin RPC server task handle (drop cancels the task)
    #[allow(dead_code)]
    admin_handle: Option<tokio::task::JoinHandle<()>>,
    /// Path to the admin RPC socket file (cleaned up on shutdown)
    admin_socket_path: PathBuf,
}
// --- Topological startup orchestration ---
impl Daemon {
    /// Resolve the deterministic startup order from the component dependency graph.
    ///
    /// Returns the layers produced by [`topo_sort_layers`].  If the dependency
    /// graph contains a cycle the daemon must refuse to start.
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
                RenderersPlugins,
                SessionConfigProvider,
                SkillsRegistry,
            ],
            vec![
                ArchiveSweeper,
                DreamingScheduler,
                IMAdapters,
                PermissionEngine,
                SkillWatcher,
                SpawnController,
                SystemPromptBuilder,
                ToolsRegistry,
            ],
            vec![ApprovalFlow, SessionManager],
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
    ) -> anyhow::Result<(
        Arc<closeclaw_agent::registry::AgentRegistry>,
        Arc<RwLock<Option<DiskSkillRegistry>>>,
        Arc<ToolRegistry>,
        SkillWatcherHandle,
    )> {
        let agent_registry = Arc::new(closeclaw_agent::registry::AgentRegistry::new());
        info!("Agent registry initialized");
        let (skill_registry, skill_watcher) =
            skill_reload::init_skill_hot_reload(config_dir).await?;
        let tool_registry = Arc::new(ToolRegistry::new());
        Ok((agent_registry, skill_registry, tool_registry, skill_watcher))
    }

    /// Phase 3: Core services — Gateway, SessionManager, IM plugins, SlashDispatcher.
    async fn init_phase_3_core_services(
        config_dir: &str,
        storage: &Arc<SqliteStorage>,
        permission_engine: &Arc<PermissionEngine>,
    ) -> anyhow::Result<(Arc<Gateway>, Arc<SessionManager>, shutdown::ShutdownHandle)> {
        let gateway_config = GatewayConfig {
            name: "closeclaw".to_string(),
            rate_limit_per_minute: 60,
            max_message_size: 16_384,
            dm_scope: DmScope::default(),
            ..Default::default()
        };
        let session_manager = Arc::new(SessionManager::new(
            &gateway_config,
            None,
            Some(PathBuf::from(config_dir)),
            Self::read_bootstrap_mode(),
            ReasoningLevel::default(),
        ));
        let processor_registry =
            Arc::new(processor_chain::build_processor_registry(&gateway_config));
        info!(
            inbound_len = processor_registry.inbound_len(),
            outbound_len = processor_registry.outbound_len(),
            "processor registry built for daemon"
        );
        let gateway = Gateway::with_processor_registry(
            gateway_config,
            Arc::clone(&session_manager),
            processor_registry,
        );
        gateway
            .set_storage(Arc::clone(storage) as Arc<dyn PersistenceService>)
            .await;

        // Run session recovery scan: load all active checkpoints, detect
        // pending_operations, and persist recovery notifications/failure
        // results into checkpoints so resolve.rs can inject them when
        // sessions are restored.
        {
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
                }
                Err(e) => {
                    tracing::warn!(error = %e, "recovery scan failed — continuing without recovery");
                }
            }
        }

        if let Err(e) = session_manager.rebuild_key_registry().await {
            tracing::warn!(error = %e, "failed to rebuild key_registry — continuing");
        }
        if let Err(e) = session_manager.rebuild_spawn_tree().await {
            tracing::warn!(error = %e, "failed to rebuild spawn_tree — continuing");
        }
        let gateway = Arc::new(gateway);
        gateway.set_self_ref(Arc::clone(&gateway));
        closeclaw_im_adapter::platforms::register_platform_plugins(&gateway, config_dir).await;
        Self::init_terminal_plugin(&gateway).await;
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
        Ok((gateway, session_manager, shutdown))
    }
}

// --- Phase 4-5 initialization ---
impl Daemon {
    /// Phase 4: Wiring — ApprovalFlow.
    async fn init_phase_4_wiring(
        gateway: &Arc<Gateway>,
        session_manager: &Arc<SessionManager>,
        permission_engine: &Arc<PermissionEngine>,
        config_manager: &Arc<closeclaw_config::ConfigManager>,
    ) -> Arc<tokio::sync::Mutex<ApprovalFlow>> {
        let approval_flow = Arc::new(tokio::sync::Mutex::new(ApprovalFlow::new(
            Arc::clone(session_manager) as Arc<dyn SessionLookup>,
            Arc::new(|_| {}),
            tokio::runtime::Handle::current(),
            HeartbeatApprovalMode::default(),
        )));
        gateway.set_approval_flow(Arc::clone(&approval_flow)).await;
        let _builtin_skills = builtin_skills_with_engine_and_approval_flow(
            Arc::clone(permission_engine),
            Arc::clone(&approval_flow),
            Some(Arc::clone(session_manager)),
            config_manager.agent_permissions(),
        );
        approval_flow
    }

    /// Phase 5: Background services — ArchiveSweeper, DreamingScheduler, registry population.
    async fn init_phase_5_background(
        config_manager: &Arc<ConfigManager>,
        agent_registry: &Arc<closeclaw_agent::registry::AgentRegistry>,
        skill_registry: &Arc<RwLock<Option<DiskSkillRegistry>>>,
        tool_registry: &Arc<ToolRegistry>,
        session_manager: &Arc<SessionManager>,
        permission_engine: &Arc<PermissionEngine>,
        data_dir: &std::path::Path,
    ) -> anyhow::Result<(
        watch::Sender<()>,
        watch::Sender<()>,
        Option<config_watcher::ConfigWatcherHandle>,
    )> {
        let (sweeper_tx, sweeper_rx) = watch::channel(());
        let (dreaming_tx, dreaming_rx) = watch::channel(());
        Self::spawn_background_services(
            config_manager,
            session_manager,
            data_dir,
            sweeper_rx,
            dreaming_rx,
        );
        // Create SpawnController as an independent component (depends on AgentRegistry).
        let spawn_controller = Arc::new(closeclaw_gateway::SpawnController::new(
            Arc::clone(config_manager),
            Arc::clone(session_manager),
            Arc::clone(permission_engine),
        ));
        let config_subdir = PathBuf::from(data_dir).join("config");
        let ctx = registries::RegistryContext {
            config_manager,
            agent_registry,
            skill_registry,
            tool_registry,
            session_manager,
            permission_engine,
            spawn_controller: Arc::clone(&spawn_controller),
            config_subdir: &config_subdir,
        };
        let config_watcher = registries::populate_registries(&ctx).await;
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
        // Inject static-layer cache invalidation callback so /system clear
        // can invalidate section caches without gateway depending on
        // closeclaw-system-prompt directly.
        session_manager
            .set_cache_invalidator(Arc::new(|| {
                invalidate_all_sections();
            }))
            .await;
        Ok((sweeper_tx, dreaming_tx, config_watcher))
    }

    /// Spawn ArchiveSweeper and DreamingScheduler background tasks.
    fn spawn_background_services(
        config_manager: &Arc<ConfigManager>,
        session_manager: &Arc<SessionManager>,
        data_dir: &std::path::Path,
        sweeper_rx: watch::Receiver<()>,
        dreaming_rx: watch::Receiver<()>,
    ) {
        let session_config_provider =
            config_manager.session_config_provider().unwrap_or_else(|| {
                tracing::warn!("session config provider not available, using defaults");
                Arc::new(
                    closeclaw_config::session::JsonSessionConfigProvider::new("/dev/null").unwrap(),
                )
            });
        let dreaming_config_provider = Arc::clone(&session_config_provider);
        let storage: Arc<dyn PersistenceService> = {
            let s = Arc::new(
                SqliteStorage::new(data_dir).expect("SqliteStorage should already be initialized"),
            );
            s as Arc<dyn PersistenceService>
        };
        let sweeper = Arc::new(
            ArchiveSweeper::new(Arc::clone(&storage), session_config_provider)
                .with_session_manager(Arc::clone(session_manager)),
        );
        let sweeper_for_task = Arc::clone(&sweeper);
        tokio::spawn(async move {
            sweeper_for_task.run(sweeper_rx).await;
        });
        info!("ArchiveSweeper spawned");
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
        // NoopMinerLlmCaller returns empty events until a real LLM caller is wired up.
        let dreaming_pipeline = Arc::new(DreamingPipeline::with_config(
            memory_config.config.dreaming.clone(),
        ));
        let memory_miner = Arc::new(MemoryMiner::new(
            closeclaw_memory::miner::MinerConfig::from_mining_config(&memory_config.config.mining),
            Box::new(noop_miner_llm::NoopMinerLlmCaller),
            data_dir.join(db_path),
            data_dir.join(md_path).to_string_lossy().into_owned(),
            String::new(),
        ));
        let dreaming_scheduler = crate::dreaming_scheduler::DreamingScheduler::new(
            storage,
            dreaming_config_provider,
            dreaming_pipeline,
            memory_miner,
        );
        tokio::spawn(async move {
            dreaming_scheduler.run(dreaming_rx).await;
        });
        info!("DreamingScheduler spawned");
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

// --- Lifecycle: start, run ---
impl Daemon {
    /// Start the daemon with the given config directory.
    pub async fn start(config_dir: &str) -> anyhow::Result<Self> {
        let permission_engine = Self::build_permission_engine(config_dir);
        Self::start_with_engine(config_dir, permission_engine).await
    }

    /// Start the daemon with a custom permission engine (useful for testing).
    pub async fn start_with_engine(
        config_dir: &str,
        permission_engine: Arc<PermissionEngine>,
    ) -> anyhow::Result<Self> {
        info!("Starting CloseClaw daemon with config_dir={}", config_dir);
        Self::load_env(config_dir);
        let (startup_layers, _phase_components) = Self::resolve_startup_order()?;
        Self::log_startup_order(&startup_layers);
        let (config_manager, storage, data_dir) = Self::init_phase_1_foundation(config_dir)?;
        let (agent_registry, skill_registry, tool_registry, skill_watcher) =
            Self::init_phase_2_registries(config_dir).await?;
        let (gateway, session_manager, shutdown) =
            Self::init_phase_3_core_services(config_dir, &storage, &permission_engine).await?;
        let shutdown = Arc::new(shutdown);
        // Wire shutdown handle into Gateway and SessionManager for
        // busy-count tracking during drain.
        let common_sh = crate::bridge::common_shutdown_handle(&shutdown);
        gateway.set_shutdown_handle(Arc::clone(&common_sh));
        session_manager.set_shutdown_handle(common_sh).await;
        let approval_flow = Self::init_phase_4_wiring(
            &gateway,
            &session_manager,
            &permission_engine,
            &config_manager,
        )
        .await;
        let (sweeper_tx, dreaming_tx, config_watcher) = Self::init_phase_5_background(
            &config_manager,
            &agent_registry,
            &skill_registry,
            &tool_registry,
            &session_manager,
            &permission_engine,
            &data_dir,
        )
        .await?;
        let (admin_handle, admin_sock_path) = Self::init_phase_6_admin_rpc(
            &agent_registry,
            &skill_registry,
            &config_manager,
            config_dir,
        )
        .await;
        info!(
            "Gateway initialized — CloseClaw daemon started successfully (v{})",
            env!("CARGO_PKG_VERSION")
        );
        Ok(Self {
            gateway,
            agent_registry,
            permission_engine,
            shutdown,
            session_manager,
            storage,
            sweeper_shutdown_tx: sweeper_tx,
            dreaming_scheduler_shutdown_tx: dreaming_tx,
            skill_registry,
            _skill_watcher: Some(skill_watcher),
            _config_watcher: config_watcher,
            approval_flow,
            admin_handle: Some(admin_handle),
            admin_socket_path: admin_sock_path,
        })
    }

    /// Run the daemon — blocks until shutdown signal is received, then
    /// executes Phase 0–7 shutdown sequence.
    pub async fn run(&mut self) -> anyhow::Result<()> {
        use tokio::signal::unix::{signal, SignalKind};

        // Phase 0: Signal reception & mode determination
        // Register signal handlers and wait for the first shutdown signal.
        let mut sigint = signal(SignalKind::interrupt())
            .map_err(|e| anyhow::anyhow!("failed to register SIGINT handler: {}", e))?;
        let mut sigterm = signal(SignalKind::terminate())
            .map_err(|e| anyhow::anyhow!("failed to register SIGTERM handler: {}", e))?;

        tokio::select! {
            _ = sigint.recv() => {
                info!("Received Ctrl+C, initiating forceful shutdown...");
                self.shutdown.try_start_forceful_shutdown();
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, initiating graceful shutdown...");
                self.shutdown.try_start_shutdown();
            }
        }

        self.phase_1_inbound_drain(&mut sigint, &mut sigterm).await;
        let mode = self.shutdown.mode();
        info!(phase = 1, "inbound shutdown complete");
        let stop_result = self.phase_2_session_stop(mode).await;
        info!(
            phase = 2,
            succeeded = stop_result.succeeded,
            failed = stop_result.failed,
            skipped = stop_result.skipped,
            "session stop complete"
        );
        self.phase_3_background_stop().await;
        info!(phase = 3, "background tasks stopped");
        self.phase_4_final_persist(mode).await;
        info!(phase = 4, "final persistence complete");
        self.phase_5_outbound_close().await;
        info!(phase = 5, "outbound shutdown complete");
        self.phase_6_storage_close().await;
        info!(phase = 6, "storage closed");
        self.phase_7_exit().await;
        info!(phase = 7, "shutdown complete — exiting");
        Ok(())
    }

    /// Phase 1: Inbound shutdown + drain.
    ///
    /// - Calls `shutdown()` on all registered IM plugins
    /// - Initiates graceful drain (waits for in-flight operations)
    /// - Monitors for escalation signals (repeated SIGTERM/SIGINT)
    async fn phase_1_inbound_drain(
        &self,
        sigint: &mut tokio::signal::unix::Signal,
        sigterm: &mut tokio::signal::unix::Signal,
    ) {
        // Shutdown inbound for all registered plugins
        let plugins = self.gateway.get_all_plugins().await;
        for plugin in &plugins {
            if let Err(e) = plugin.shutdown_inbound().await {
                tracing::warn!(
                    platform = plugin.platform(),
                    error = %e,
                    "failed to shutdown plugin inbound — continuing"
                );
            }
        }

        // Initiate graceful drain
        let shutdown_handle = self.shutdown.clone();
        let mut shutdown_task = tokio::spawn(async move {
            shutdown_handle.initiate_shutdown().await;
        });

        // Monitor for escalation signals during drain
        loop {
            tokio::select! {
                result = &mut shutdown_task => {
                    if let Err(e) = result {
                        tracing::error!(error = %e, "shutdown task panicked");
                    }
                    break;
                }
                _ = sigint.recv() => {
                    if self.shutdown.escalate_to_forceful() {
                        info!("Received repeated Ctrl+C, escalated to forceful shutdown");
                    }
                }
                _ = sigterm.recv() => {
                    if self.shutdown.escalate_to_forceful() {
                        info!("Received repeated SIGTERM, escalated to forceful shutdown");
                    }
                }
            }
        }
    }

    /// Phase 2: Session stop (leaf → root) with progress card updates.
    ///
    /// Sends a progress notification card at the start, monitors for
    /// graceful → forceful escalation to update the card, and sends a
    /// final card when all sessions have stopped.
    async fn phase_2_session_stop(
        &self,
        mode: crate::shutdown::ShutdownMode,
    ) -> closeclaw_gateway::session_manager::stop::StopResult {
        // Send initial progress card (no-op if no active sessions)
        self.gateway.send_shutdown_progress_card(mode).await;

        // Create progress channel for real-time session stop updates
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<
            closeclaw_gateway::session_manager::stop::StopProgress,
        >(64);

        // Spawn session stop as a background task
        let sm = self.gateway.session_manager().clone();
        let mut stop_handle =
            tokio::spawn(async move { sm.stop_all_sessions(mode, Some(&progress_tx)).await });

        // Spawn fresh signal handlers for escalation monitoring during Phase 2.
        // Phase 1's handlers are consumed by its tokio::select! loop.
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt()).ok();
        let mut sigterm = signal(SignalKind::terminate()).ok();

        // Monitor for escalation and update card
        let mut last_mode = mode;
        let mut stop_completed = false;
        let mut stop_result = None;
        let mut last_card_update = std::time::Instant::now();
        let throttle_interval = std::time::Duration::from_secs(2);

        while !stop_completed {
            tokio::select! {
                biased;

                result = &mut stop_handle => {
                    // Session stop complete
                    match result {
                        Ok(sr) => {
                            stop_result = Some(sr);
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "session stop task panicked");
                            stop_result = Some(
                                closeclaw_gateway::session_manager::stop::StopResult::default()
                            );
                        }
                    }
                    stop_completed = true;
                }

                Some(progress) = progress_rx.recv() => {
                    // Progress event: update card with throttle
                    let now = std::time::Instant::now();
                    if progress.remaining == 0
                        || now.duration_since(last_card_update) >= throttle_interval
                    {
                        let current_mode: closeclaw_common::shutdown::ShutdownMode =
                            self.shutdown.mode();
                        self.gateway
                            .send_shutdown_progress_card(current_mode)
                            .await;
                        last_card_update = now;
                    }
                }

                _ = async {
                    // Wait for any escalation signal regardless of
                    // which handlers are available.
                    let escalate = || {
                        if self.shutdown.escalate_to_forceful() {
                            info!("Phase 2: escalated to forceful shutdown");
                        }
                    };
                    match (&mut sigint, &mut sigterm) {
                        (Some(i), Some(t)) => {
                            tokio::select! {
                                _ = i.recv() => escalate(),
                                _ = t.recv() => escalate(),
                            }
                        }
                        (Some(i), None) => { let _ = i.recv().await; escalate(); }
                        (None, Some(t)) => { let _ = t.recv().await; escalate(); }
                        (None, None) => { std::future::pending::<()>().await; }
                    }
                } => {
                    // Escalation signal received
                }
            }

            // Check if mode changed and update card
            let current_mode: closeclaw_common::shutdown::ShutdownMode = self.shutdown.mode();
            if current_mode != last_mode {
                tracing::info!(
                    ?last_mode,
                    ?current_mode,
                    "shutdown mode changed, updating progress card"
                );
                self.gateway.send_shutdown_progress_card(current_mode).await;
                last_card_update = std::time::Instant::now();
                last_mode = current_mode;
            }
        }

        // Session stop completed — send final card
        let result = stop_result.unwrap_or_default();
        self.gateway.send_shutdown_final_card(&result).await;
        result
    }

    /// Phase 3: Background task stop.
    ///
    /// - Drops SkillWatcher and ConfigWatcher (RAII) via `take()`
    /// - Signals ArchiveSweeper and DreamingScheduler to stop
    /// - Clears pending approval requests
    async fn phase_3_background_stop(&mut self) {
        // SkillWatcher and ConfigWatcher are RAII — stop on drop.
        // Explicitly take() and drop here to match Phase 3 ordering
        // in the design doc, rather than waiting for Daemon destruction.
        if let Some(watcher) = self._skill_watcher.take() {
            drop(watcher);
            tracing::info!("SkillWatcher dropped in Phase 3");
        }
        if let Some(watcher) = self._config_watcher.take() {
            drop(watcher);
            tracing::info!("ConfigWatcher dropped in Phase 3");
        }

        // Signal ArchiveSweeper to stop
        let _ = self.sweeper_shutdown_tx.send(());
        // Signal DreamingScheduler to stop
        let _ = self.dreaming_scheduler_shutdown_tx.send(());
        // Clear pending approval requests (denied with callbacks triggered)
        self.approval_flow.lock().await.clear();
    }

    /// Phase 4: Final persistence — flush checkpoints and sync WAL.
    async fn phase_4_final_persist(&self, mode: crate::shutdown::ShutdownMode) {
        match self.gateway.flush_all_sessions(mode).await {
            Ok(n) => tracing::info!(count = n, mode = ?mode, "flushed session checkpoints"),
            Err(e) => tracing::warn!(error = %e, "failed to flush sessions"),
        }
        match self.gateway.sync_storage().await {
            Ok(()) => tracing::info!("storage fsync complete"),
            Err(e) => tracing::warn!(error = %e, "storage fsync failed"),
        }
    }

    /// Phase 5: Outbound shutdown — clean up routing tables.
    async fn phase_5_outbound_close(&self) {
        self.gateway.close_outbound().await;
    }

    /// Phase 6: Storage close — release persistent connections/handles.
    async fn phase_6_storage_close(&self) {
        match self.gateway.close_storage().await {
            Ok(()) => tracing::info!("storage closed"),
            Err(e) => tracing::warn!(error = %e, "storage close failed"),
        }
    }

    /// Phase 7: Exit cleanup — log warnings, remove admin socket.
    async fn phase_7_exit(&self) {
        // Check for sessions still in the active table — after
        // stop_all_sessions, only sessions that were NOT stopped
        // (e.g. skipped due to missing ConversationSession) remain.
        let remaining = self.gateway.session_manager().get_all_sessions().await;
        let mut stopped_count = 0usize;
        for session in &remaining {
            // Only warn about sessions that haven't been stopped yet.
            let is_stopped = {
                let conv = self
                    .gateway
                    .session_manager()
                    .conversation_sessions
                    .read()
                    .await;
                match conv.get(&session.id) {
                    Some(cs) => cs.read().await.is_stopped(),
                    None => false,
                }
            };
            if is_stopped {
                stopped_count += 1;
            } else {
                tracing::warn!(
                    session_id = %session.id,
                    "session still active and not stopped at exit — may need manual recovery"
                );
            }
        }
        if !remaining.is_empty() {
            tracing::info!(
                remaining = remaining.len(),
                stopped = stopped_count,
                "phase 7: session table state at exit"
            );
        }
        // Clean up admin socket file
        let _ = tokio::fs::remove_file(&self.admin_socket_path).await;
    }
}

// --- Config loading helpers ---
impl Daemon {
    /// Load .env file from config_dir if it exists.
    fn load_env(config_dir: &str) {
        let env_path = std::path::Path::new(config_dir).join(".env");
        if env_path.exists() {
            if let Err(e) = load_env_file(&env_path) {
                tracing::warn!(error = %e, path = %env_path.display(), "failed to load .env file");
            } else {
                info!("Loaded environment from {}", env_path.display());
            }
        }
    }

    /// Read BOOTSTRAP_MODE env var and convert to BootstrapMode.
    /// "minimal" → Minimal, anything else (including absent) → Full.
    fn read_bootstrap_mode() -> BootstrapMode {
        match std::env::var("BOOTSTRAP_MODE").as_deref() {
            Ok("minimal") => BootstrapMode::Minimal,
            _ => BootstrapMode::Full,
        }
    }

    /// Build permission engine, loading templates from config_dir/templates/ if present.
    fn build_permission_engine(config_dir: &str) -> Arc<PermissionEngine> {
        let rule_set = RuleSet {
            rules: Vec::new(),
            defaults: Defaults::default(),
            template_includes: Vec::new(),
            agent_creators: std::collections::HashMap::new(),
        };
        let mut engine = PermissionEngine::new(rule_set, PathBuf::from(config_dir));
        let templates_dir = std::path::Path::new(config_dir).join("templates");
        if templates_dir.exists() {
            if let Ok(templates) =
                closeclaw_permission::templates::load_templates_from_dir(&templates_dir)
            {
                let count = templates.len();
                if count > 0 {
                    engine.load_templates(templates);
                    info!(
                        "Loaded {} permission templates from {}",
                        count,
                        templates_dir.display()
                    );
                }
            }
        }
        info!("Permission engine initialized");
        Arc::new(engine)
    }

    /// Migrate legacy openclaw.json if present (non-fatal on error).
    fn run_config_migration(config_dir: &str) {
        let openclaw_json_path = Path::new(config_dir).join("openclaw.json");
        info!("Checking for legacy openclaw.json migration...");
        match migrate_if_needed(&openclaw_json_path, config_dir) {
            Ok(true) => info!("Legacy openclaw.json migration completed successfully"),
            Ok(false) => info!("No migration needed — config directory is up to date"),
            Err(e) => tracing::warn!(
                error = %e,
                "openclaw.json migration failed — continuing with existing config"
            ),
        }
    }
}

// --- Service init helpers ---
impl Daemon {
    /// Initialize the terminal (CLI) IM plugin and register with Gateway.
    async fn init_terminal_plugin(gateway: &Arc<Gateway>) {
        let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(TerminalPlugin::new());
        gateway.register_plugin(plugin).await;
        info!("Terminal plugin registered");
    }

    /// Initialize the slash command dispatcher and register all handlers.
    async fn init_slash_dispatcher(
        gateway: &Arc<Gateway>,
        session_manager: &Arc<SessionManager>,
        permission_engine: &Arc<PermissionEngine>,
    ) {
        let slash_registry = Arc::new(HandlerRegistry::new());
        slash_registry.register(Arc::new(CompactHandler));
        slash_registry.register(Arc::new(ClearHandler::new(Arc::clone(session_manager))));
        slash_registry.register(Arc::new(ExecHandler));
        slash_registry.register(Arc::new(WorkdirHandler::new(Arc::clone(session_manager))));
        let help_handler = HelpHandler::new(Arc::clone(&slash_registry));
        slash_registry.register(Arc::new(help_handler));
        slash_registry.register(Arc::new(ReasoningHandler::new(Arc::clone(session_manager))));
        slash_registry.register(Arc::new(VerboseHandler::new(Arc::clone(session_manager))));
        slash_registry.register(Arc::new(SystemHandler::new(Arc::clone(session_manager))));
        slash_registry.register(Arc::new(NewSessionHandler));
        slash_registry.register(Arc::new(StopHandler));
        slash_registry.register(Arc::new(StatusHandler::new(Arc::clone(session_manager))));
        let slash_dispatcher = Arc::new(crate::bridge::SlashDispatcherWrapper(
            SlashDispatcher::from_shared(slash_registry),
        ));
        gateway
            .set_slash_dispatcher(slash_dispatcher as Arc<dyn closeclaw_common::SlashRouter>)
            .await;
        // 高危 slash 指令（如 /exec）需要权限引擎介入；在此注入使得
        // dispatch_slash 在 Branch 2 时能取到 engine。
        gateway
            .set_permission_engine(Arc::clone(permission_engine))
            .await;
        info!("Slash dispatcher installed");
    }
}

#[cfg(test)]
mod dreaming_scheduler_tests;
#[cfg(test)]
mod shutdown_alignment_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod unit_tests;
