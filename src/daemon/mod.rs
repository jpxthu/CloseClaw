//! Daemon - CloseClaw background service
//!
//! Orchestrates all components: Gateway, AgentRegistry, PermissionEngine.
//! Handles graceful shutdown via ShutdownCoordinator.
pub mod config_reload;
pub mod dreaming_scheduler;
pub mod registries;
pub mod shutdown;
pub mod skill_reload;
pub mod startup;
use crate::admin::client::admin_socket_path;
use crate::admin::server::{AdminContext, AdminServer};
use crate::config::migration::migrate_if_needed;
use crate::config::providers::ConfigProvider;
use crate::config::ConfigManager;
use crate::daemon::startup::{all_component_entries, topo_sort_layers, StartupError};

/// Resolved startup plan: topo-sort layers plus validated phase components.
/// Each outer element is a layer/phase; each inner element is a [`ComponentId`].
type StartupPlan = (
    Vec<Vec<crate::daemon::startup::ComponentId>>,
    Vec<Vec<crate::daemon::startup::ComponentId>>,
);
use crate::gateway::{DmScope, Gateway, GatewayConfig, SessionManager};
use crate::im::feishu::{FeishuAdapter, FeishuPlugin};
use crate::im::terminal::TerminalPlugin;
use crate::renderer::feishu::FeishuRenderer;
use crate::slash::dispatcher::SlashDispatcher;
use crate::slash::handlers::{ReasoningHandler, SystemHandler, WorkdirHandler};
use crate::slash::registry::HandlerRegistry;
use crate::slash::{
    ClearHandler, CompactHandler, ExecHandler, HelpHandler, NewSessionHandler, StatusHandler,
    StopHandler,
};

use crate::memory::dreaming::DreamingPipeline;
use crate::memory::miner::MemoryMiner;
use crate::permission::approval_flow::ApprovalFlow;
use crate::permission::{Defaults, PermissionEngine, RuleSet};
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::PersistenceService;
use crate::session::persistence::ReasoningLevel;
use crate::session::storage::SqliteStorage;
use crate::session::sweeper::ArchiveSweeper;
use crate::skills::builtin::builtin_skills_with_engine_and_approval_flow;
use crate::skills::{DiskSkillRegistry, SkillWatcherHandle};
use crate::tools::ToolRegistry;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::sync::watch;
use tracing::info;

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
        std::env::set_var(&key, &value);
    }
    Ok(())
}
mod llm_init;

/// Global daemon state
pub struct Daemon {
    pub gateway: Arc<Gateway>,
    pub agent_registry: Arc<crate::agent::registry::AgentRegistry>,
    pub permission_engine: Arc<PermissionEngine>,
    pub shutdown: shutdown::ShutdownHandle,
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
    _config_watcher: Option<config_reload::ConfigWatcherHandle>,
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
        layers: &[Vec<crate::daemon::startup::ComponentId>],
    ) -> Result<Vec<Vec<crate::daemon::startup::ComponentId>>, StartupError> {
        use crate::daemon::startup::ComponentId::*;
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
                SystemPromptBuilder,
                ToolsRegistry,
            ],
            vec![ApprovalFlow, SessionManager],
            vec![Gateway],
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
    fn log_startup_order(layers: &[Vec<crate::daemon::startup::ComponentId>]) {
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
        Arc<crate::agent::registry::AgentRegistry>,
        Arc<RwLock<Option<DiskSkillRegistry>>>,
        Arc<ToolRegistry>,
        SkillWatcherHandle,
    )> {
        let agent_registry = Arc::new(crate::agent::registry::AgentRegistry::new());
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
        let gateway = Gateway::new(gateway_config, Arc::clone(&session_manager));
        gateway
            .set_storage(Arc::clone(storage) as Arc<dyn PersistenceService>)
            .await;
        if let Err(e) = session_manager.rebuild_key_registry().await {
            tracing::warn!(error = %e, "failed to rebuild key_registry — continuing");
        }
        if let Err(e) = session_manager.rebuild_spawn_tree().await {
            tracing::warn!(error = %e, "failed to rebuild spawn_tree — continuing");
        }
        let gateway = Arc::new(gateway);
        gateway.set_self_ref(Arc::clone(&gateway));
        Self::init_feishu_plugin(config_dir, &gateway).await?;
        Self::init_terminal_plugin(&gateway).await;
        Self::init_slash_dispatcher(&gateway, &session_manager, permission_engine).await;
        let shutdown = shutdown::ShutdownHandle::new();
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
    ) -> Arc<tokio::sync::Mutex<ApprovalFlow>> {
        let approval_flow = Arc::new(tokio::sync::Mutex::new(ApprovalFlow::new(
            Arc::clone(session_manager),
            Arc::new(|_| {}),
            tokio::runtime::Handle::current(),
        )));
        gateway.set_approval_flow(Arc::clone(&approval_flow)).await;
        let _builtin_skills = builtin_skills_with_engine_and_approval_flow(
            Arc::clone(permission_engine),
            Arc::clone(&approval_flow),
        );
        approval_flow
    }

    /// Phase 5: Background services — ArchiveSweeper, DreamingScheduler, registry population.
    async fn init_phase_5_background(
        config_manager: &Arc<ConfigManager>,
        agent_registry: &Arc<crate::agent::registry::AgentRegistry>,
        skill_registry: &Arc<RwLock<Option<DiskSkillRegistry>>>,
        tool_registry: &Arc<ToolRegistry>,
        session_manager: &Arc<SessionManager>,
        permission_engine: &Arc<PermissionEngine>,
        data_dir: &std::path::Path,
    ) -> anyhow::Result<(
        watch::Sender<()>,
        watch::Sender<()>,
        Option<config_reload::ConfigWatcherHandle>,
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
        let config_subdir = PathBuf::from(data_dir).join("config");
        let config_watcher = Self::populate_registries(
            config_manager,
            agent_registry,
            skill_registry,
            tool_registry,
            session_manager,
            permission_engine,
            &config_subdir,
        )
        .await;
        session_manager
            .set_tool_registry(Arc::clone(tool_registry))
            .await;
        session_manager
            .set_skill_registry(skill_registry.clone())
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
                    crate::config::session::JsonSessionConfigProvider::new("/dev/null").unwrap(),
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
        let dreaming_pipeline = Arc::new(DreamingPipeline::new());
        let memory_miner = Arc::new(MemoryMiner::new());
        let dreaming_scheduler = crate::daemon::dreaming_scheduler::DreamingScheduler::new(
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
        let approval_flow =
            Self::init_phase_4_wiring(&gateway, &session_manager, &permission_engine).await;
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
        let admin_sock_path = admin_socket_path(Path::new(config_dir));
        let admin_context = AdminContext {
            agent_registry: Arc::clone(&agent_registry),
            skill_registry: skill_registry.clone(),
            config_manager: Arc::clone(&config_manager),
            config_dir: PathBuf::from(config_dir),
        };
        let admin_server = AdminServer::new(&admin_sock_path, admin_context);
        let admin_handle = tokio::spawn(async move {
            if let Err(e) = admin_server.serve().await {
                tracing::error!(error = %e, "admin RPC server failed");
            }
        });
        info!("admin RPC server started on {}", admin_sock_path.display());
        info!(
            "Gateway initialized — CloseClaw daemon started successfully (v{})",
            env!("CARGO_PKG_VERSION")
        );
        Ok(Self {
            gateway,
            agent_registry,
            permission_engine,
            shutdown,
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

    /// Run the daemon — blocks until shutdown signal is received.
    ///
    /// First signal triggers graceful shutdown; subsequent signals escalate to forceful.
    pub async fn run(&self) -> anyhow::Result<()> {
        use tokio::signal::unix::{signal, SignalKind};

        // Set up signal listeners for SIGTERM and SIGINT
        let mut sigint = signal(SignalKind::interrupt())
            .map_err(|e| anyhow::anyhow!("failed to register SIGINT handler: {}", e))?;
        let mut sigterm = signal(SignalKind::terminate())
            .map_err(|e| anyhow::anyhow!("failed to register SIGTERM handler: {}", e))?;

        // Wait for the first shutdown signal
        tokio::select! {
            _ = sigint.recv() => {
                info!("Received Ctrl+C, initiating graceful shutdown...");
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, initiating graceful shutdown...");
            }
        }

        // Start graceful shutdown in a background task
        let shutdown_handle = self.shutdown.clone();
        let mut shutdown_task = tokio::spawn(async move {
            shutdown_handle.initiate_shutdown().await;
        });

        // Monitor for escalation signals during graceful drain
        loop {
            tokio::select! {
                result = &mut shutdown_task => {
                    // Shutdown drain completed (graceful or forceful)
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

        // Flush all active session checkpoints before shutdown
        let mode = self.shutdown.mode();
        match self.gateway.flush_all_sessions(mode).await {
            Ok(n) => tracing::info!(count = n, mode = ?mode, "flushed session checkpoints"),
            Err(e) => tracing::warn!(error = %e, "failed to flush sessions"),
        }
        // Clear all pending approval requests (denied with callbacks triggered)
        self.approval_flow.lock().await.clear();
        let _ = self.sweeper_shutdown_tx.send(());
        let _ = self.dreaming_scheduler_shutdown_tx.send(());
        // Clean up admin socket file
        let _ = tokio::fs::remove_file(&self.admin_socket_path).await;
        Ok(())
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
                crate::permission::templates::load_templates_from_dir(&templates_dir)
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
    /// Initialize Feishu IM plugin from env or config.
    async fn init_feishu_plugin(_config_dir: &str, gateway: &Arc<Gateway>) -> anyhow::Result<()> {
        let app_id = std::env::var("FEISHU_APP_ID").ok();
        let app_secret = std::env::var("FEISHU_APP_SECRET").ok();
        let verification_token = std::env::var("FEISHU_VERIFICATION_TOKEN").ok();
        if let (Some(app_id), Some(app_secret), Some(verification_token)) =
            (app_id, app_secret, verification_token)
        {
            let adapter = Arc::new(FeishuAdapter::new(app_id, app_secret, verification_token));
            let renderer = Arc::new(FeishuRenderer::new());
            let plugin: Arc<dyn crate::im::IMPlugin> =
                Arc::new(FeishuPlugin::new(adapter, renderer));
            gateway.register_plugin(plugin).await;
            info!("Feishu plugin registered");
        } else {
            info!("Feishu credentials not found in env — Feishu plugin not registered");
        }
        Ok(())
    }

    /// Initialize the terminal (CLI) IM plugin and register with Gateway.
    async fn init_terminal_plugin(gateway: &Arc<Gateway>) {
        let plugin: Arc<dyn crate::im::IMPlugin> = Arc::new(TerminalPlugin::new());
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
        slash_registry.register(Arc::new(SystemHandler::new(Arc::clone(session_manager))));
        slash_registry.register(Arc::new(NewSessionHandler));
        slash_registry.register(Arc::new(StopHandler));
        slash_registry.register(Arc::new(StatusHandler::new(Arc::clone(session_manager))));
        let slash_dispatcher = Arc::new(SlashDispatcher::from_shared(slash_registry));
        gateway.set_slash_dispatcher(slash_dispatcher).await;
        // 高危 slash 指令（如 /exec）需要权限引擎介入；在此注入使得
        // dispatch_slash 在 Branch 2 时能取到 engine。
        gateway
            .set_permission_engine(Arc::clone(permission_engine))
            .await;
        info!("Slash dispatcher installed");
    }

    /// Populate AgentRegistry, SkillRegistry, ToolRegistry, and wire them into
    /// SessionManager.  Also starts ConfigHotReload watcher.
    #[allow(dead_code)]
    async fn populate_registries(
        config_manager: &Arc<ConfigManager>,
        agent_registry: &Arc<crate::agent::registry::AgentRegistry>,
        skill_registry: &Arc<RwLock<Option<DiskSkillRegistry>>>,
        tool_registry: &Arc<ToolRegistry>,
        session_manager: &Arc<SessionManager>,
        permission_engine: &Arc<PermissionEngine>,
        config_subdir: &Path,
    ) -> Option<config_reload::ConfigWatcherHandle> {
        let ctx = registries::RegistryContext {
            config_manager,
            agent_registry,
            skill_registry,
            tool_registry,
            session_manager,
            permission_engine,
            config_subdir,
        };
        registries::populate_registries(&ctx).await
    }
}

#[cfg(test)]
mod dreaming_scheduler_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod unit_tests;
