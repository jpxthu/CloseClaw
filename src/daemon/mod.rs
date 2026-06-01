//! Daemon - CloseClaw background service
//!
//! Orchestrates all components: Gateway, AgentRegistry, PermissionEngine.
//! Handles graceful shutdown via ShutdownCoordinator.
pub mod shutdown;
pub mod skill_reload;
use crate::config::agents::AgentsConfigProvider;
use crate::config::migration::migrate_if_needed;
use crate::config::providers::ConfigProvider;
use crate::config::session::{JsonSessionConfigProvider, SessionConfigProvider};
use crate::gateway::{DmScope, Gateway, GatewayConfig, SessionManager};
use crate::im::feishu::FeishuAdapter;
use crate::slash::dispatcher::SlashDispatcher;
use crate::slash::handlers::{ClearHandler, CompactHandler, HelpHandler};
use crate::slash::registry::HandlerRegistry;

use crate::permission::approval_flow::ApprovalFlow;
use crate::permission::{Defaults, PermissionEngine, RuleSet};
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::PersistenceService;
use crate::session::persistence::ReasoningLevel;
use crate::session::storage::SqliteStorage;
use crate::session::sweeper::ArchiveSweeper;
use crate::skills::builtin::builtin_skills_with_engine_and_approval_flow;
use crate::skills::{DiskSkillRegistry, SkillWatcherHandle};
use crate::tools::builtin::register_builtin_tools;
use crate::tools::ToolRegistry;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::sync::watch;
use tracing::info;
/// Load key=value pairs from a .env file and set them as environment variables.
/// Lines starting with # are treated as comments and ignored.
mod llm_init;

fn load_env_file(path: &std::path::Path) -> std::io::Result<()> {
    let content = std::fs::read_to_string(path)?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(pos) = line.find('=') {
            let key = line[..pos].trim().to_string();
            let value = line[pos + 1..].trim().to_string();
            if !key.is_empty() && !value.is_empty() {
                std::env::set_var(&key, &value);
            }
        }
    }
    Ok(())
}
/// Global daemon state
pub struct Daemon {
    pub gateway: Arc<Gateway>,
    pub agent_registry: Arc<RwLock<crate::agent::registry::AgentRegistry>>,
    pub permission_engine: Arc<PermissionEngine>,
    pub shutdown: shutdown::ShutdownHandle,
    /// SQLite storage for session persistence
    pub storage: Arc<SqliteStorage>,
    /// Shutdown sender for ArchiveSweeper
    pub sweeper_shutdown_tx: watch::Sender<()>,
    /// Shared skill registry, updated on hot reload
    pub skill_registry: Arc<RwLock<Option<DiskSkillRegistry>>>,
    /// Skill file watcher handle (RAII: stops on drop)
    _skill_watcher: Option<SkillWatcherHandle>,
    /// Daemon-level approval orchestrator
    pub approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
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
        // Migrate legacy openclaw.json to config/ directory if needed
        let openclaw_json_path = Path::new(config_dir).join("openclaw.json");
        info!("Checking for legacy openclaw.json migration...");
        match migrate_if_needed(&openclaw_json_path, config_dir) {
            Ok(true) => {
                info!("Legacy openclaw.json migration completed successfully");
            }
            Ok(false) => {
                info!("No migration needed — config directory is up to date");
            }
            Err(e) => {
                // Migration errors are non-fatal; log and continue
                tracing::warn!(error = %e, "openclaw.json migration failed — continuing with existing config");
            }
        }
        // Initialize SQLite storage — fail hard if this fails (no MemoryStorage fallback)
        let data_dir = Path::new(config_dir);
        let storage = Arc::new(
            SqliteStorage::new(data_dir)
                .map_err(|e| anyhow::anyhow!("failed to initialize SqliteStorage: {}", e))?,
        );
        info!("SqliteStorage initialized at {}", data_dir.display());
        // Initialize session config provider — warn and use defaults if file is missing
        let session_config_path = data_dir.join("session_config.json");
        let config_provider: Arc<dyn SessionConfigProvider> =
            match JsonSessionConfigProvider::new(&session_config_path) {
                Ok(provider) => {
                    info!(
                        "Session config loaded from {}",
                        session_config_path.display()
                    );
                    Arc::new(provider)
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        path = %session_config_path.display(),
                        "failed to load session config, using hardcoded defaults"
                    );
                    // Use a non-existent path so new() returns Ok with config=None (hardcoded defaults)
                    Arc::new(
                        JsonSessionConfigProvider::new(
                            "/dev/null/closeclaw_session_config_nonexistent",
                        )
                        .unwrap(),
                    )
                }
            };
        // Create ArchiveSweeper shutdown channel and spawn sweeper
        let (sweeper_tx, sweeper_rx) = watch::channel(());
        let sweeper = Arc::new(ArchiveSweeper::new(
            Arc::clone(&storage) as Arc<dyn PersistenceService>,
            Arc::clone(&config_provider),
        ));
        let sweeper_for_task = Arc::clone(&sweeper);
        tokio::spawn(async move {
            sweeper_for_task.run(sweeper_rx).await;
        });
        info!("ArchiveSweeper spawned");
        let agent_registry = Arc::new(RwLock::new(crate::agent::registry::AgentRegistry::new(30)));
        info!("Agent registry initialized",);
        let gateway_config = GatewayConfig {
            name: "closeclaw".to_string(),
            rate_limit_per_minute: 60,
            max_message_size: 16_384,
            dm_scope: DmScope::default(),
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
            .set_storage(Arc::clone(&storage) as Arc<dyn PersistenceService>)
            .await;
        let gateway = Arc::new(gateway);

        // ── Slash Dispatcher ────────────────────────────────────────────────
        let slash_registry = Arc::new(HandlerRegistry::new());
        slash_registry.register(Arc::new(CompactHandler));
        slash_registry.register(Arc::new(ClearHandler::new(Arc::clone(&session_manager))));
        let help_handler = HelpHandler::new(Arc::clone(&slash_registry));
        slash_registry.register(Arc::new(help_handler));
        let slash_dispatcher = Arc::new(SlashDispatcher::from_shared(slash_registry));
        gateway.set_slash_dispatcher(slash_dispatcher).await;
        info!("Slash dispatcher installed");

        info!("Gateway initialized");
        Self::init_feishu_adapter(config_dir, &gateway).await?;
        let shutdown = shutdown::ShutdownHandle::new();
        info!("Shutdown coordinator initialized");
        // Initialize skill hot reload system
        let (skill_registry, skill_watcher) =
            skill_reload::init_skill_hot_reload(config_dir).await?;

        // Create ToolRegistry and register builtin tools
        let tool_registry = Arc::new(ToolRegistry::new());
        {
            let disk_reg_opt = {
                let guard = skill_registry.read().unwrap();
                guard.as_ref().map(|dr| Arc::new(dr.clone()))
            };
            if let Some(disk_reg) = disk_reg_opt {
                register_builtin_tools(&tool_registry, disk_reg, Arc::clone(&permission_engine))
                    .await;
            }
        }

        // Inject ToolRegistry and SkillRegistry into SessionManager
        session_manager.set_tool_registry(tool_registry).await;
        session_manager
            .set_skill_registry(skill_registry.clone())
            .await;

        let approval_flow = Arc::new(tokio::sync::Mutex::new(ApprovalFlow::new(
            Arc::clone(&session_manager),
            Arc::new(|_| {}),
            tokio::runtime::Handle::current(),
        )));

        // Connect approval flow to gateway
        gateway.set_approval_flow(Arc::clone(&approval_flow)).await;

        // Create builtin skills with approval flow injected
        let _builtin_skills = builtin_skills_with_engine_and_approval_flow(
            Arc::clone(&permission_engine),
            Arc::clone(&approval_flow),
        );

        info!(
            "CloseClaw daemon started successfully (v{})",
            env!("CARGO_PKG_VERSION")
        );
        Ok(Self {
            gateway,
            agent_registry,
            permission_engine,
            shutdown,
            storage,
            sweeper_shutdown_tx: sweeper_tx,
            skill_registry,
            _skill_watcher: Some(skill_watcher),
            approval_flow,
        })
    }
    /// Run the daemon — blocks until shutdown signal is received.
    #[cfg(unix)]
    pub async fn run(&self) -> anyhow::Result<()> {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt())?;
        let mut sigterm = signal(SignalKind::terminate())?;
        tokio::select! {
            _ = sigint.recv() => {
                info!("Received Ctrl+C, initiating shutdown...");
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, initiating graceful shutdown...");
            }
        }
        self.shutdown.initiate_shutdown().await;
        // Flush all active session checkpoints before shutdown
        match self.gateway.flush_all_sessions().await {
            Ok(n) => tracing::info!(count = n, "flushed session checkpoints"),
            Err(e) => tracing::warn!(error = %e, "failed to flush sessions"),
        }
        // Clear all pending approval requests (denied with callbacks triggered)
        self.approval_flow.lock().await.clear();
        let _ = self.sweeper_shutdown_tx.send(());
        Ok(())
    }
    /// Run the daemon on non-Unix platforms (falls back to Ctrl+C only).
    #[cfg(not(unix))]
    pub async fn run(&self) -> anyhow::Result<()> {
        tokio::signal::ctrl_c().await?;
        info!("Received Ctrl+C, initiating shutdown...");
        self.shutdown.initiate_shutdown().await;
        // Flush all active session checkpoints before shutdown
        match self.gateway.flush_all_sessions().await {
            Ok(n) => tracing::info!(count = n, "flushed session checkpoints"),
            Err(e) => tracing::warn!(error = %e, "failed to flush sessions"),
        }
        // Clear all pending approval requests (denied with callbacks triggered)
        self.approval_flow.lock().await.clear();
        let _ = self.sweeper_shutdown_tx.send(());
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
    /// Load and validate agents.json
    #[allow(dead_code)]
    fn load_agents_config(config_dir: &str) -> anyhow::Result<AgentsConfigProvider> {
        let path = format!("{}/agents.json", config_dir);
        let provider = AgentsConfigProvider::new(&path)
            .map_err(|e| anyhow::anyhow!("Failed to load {}: {}", path, e))?;
        provider
            .validate()
            .map_err(|e| anyhow::anyhow!("Invalid agents config: {}", e))?;
        info!("Loaded agents config from {}", path);
        Ok(provider)
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
}
// --- Service init helpers ---
impl Daemon {
    /// Initialize Feishu adapter from env or config.
    async fn init_feishu_adapter(_config_dir: &str, gateway: &Arc<Gateway>) -> anyhow::Result<()> {
        let app_id = std::env::var("FEISHU_APP_ID").ok();
        let app_secret = std::env::var("FEISHU_APP_SECRET").ok();
        let verification_token = std::env::var("FEISHU_VERIFICATION_TOKEN").ok();
        if let (Some(app_id), Some(app_secret), Some(verification_token)) =
            (app_id, app_secret, verification_token)
        {
            let adapter = Arc::new(FeishuAdapter::new(app_id, app_secret, verification_token));
            gateway
                .register_adapter("feishu".to_string(), adapter)
                .await;
            info!("Feishu adapter registered");
        } else {
            info!("Feishu credentials not found in env — Feishu adapter not registered");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod unit_tests;
