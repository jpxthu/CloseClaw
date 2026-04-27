//! Daemon - CloseClaw background service
//!
//! Orchestrates all components: Gateway, AgentRegistry, PermissionEngine.
//! Handles graceful shutdown via ShutdownCoordinator.

pub mod shutdown;

use crate::audit::{AuditEventBuilder, AuditEventType, AuditLogger, AuditResult};
use crate::chat::ChatServer;
use crate::config::agents::AgentsConfigProvider;
use crate::config::migration::migrate_if_needed;
use crate::config::providers::ConfigProvider;
use crate::config::session::{JsonSessionConfigProvider, SessionConfigProvider};
use crate::gateway::{DmScope, Gateway, GatewayConfig};
use crate::im::feishu::FeishuAdapter;
use crate::llm::{AnthropicProvider, LLMRegistry, MiniMaxProvider, OpenAIProvider};
use crate::permission::{Defaults, PermissionEngine, RuleSet};
use crate::session::persistence::PersistenceService;
use crate::session::storage::SqliteStorage;
use crate::session::sweeper::ArchiveSweeper;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::watch;
use tokio::sync::RwLock;
use tracing::info;

/// Load key=value pairs from a .env file and set them as environment variables.
/// Lines starting with # are treated as comments and ignored.
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
    /// Chat TCP server (runs as background task)
    pub chat_server: Arc<ChatServer>,
    /// Audit logger for structured audit logging
    pub audit_logger: Arc<AuditLogger>,
    /// SQLite storage for session persistence
    pub storage: Arc<SqliteStorage>,
    /// Shutdown sender for ArchiveSweeper
    pub sweeper_shutdown_tx: watch::Sender<()>,
}

// --- Lifecycle: start, run ---

impl Daemon {
    /// Start the daemon with the given config directory
    pub async fn start(config_dir: &str) -> anyhow::Result<Self> {
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

        let agents_config = Self::load_agents_config(config_dir)?;
        let permission_engine = Self::build_permission_engine(config_dir);
        let agent_registry = Arc::new(RwLock::new(crate::agent::registry::AgentRegistry::new(30)));
        info!(
            "Agent registry initialized ({} agents)",
            agents_config.agents().len()
        );

        let mut gateway = Gateway::new(GatewayConfig {
            name: "closeclaw".to_string(),
            rate_limit_per_minute: 60,
            max_message_size: 16_384,
            dm_scope: DmScope::default(),
        });
        gateway.set_storage(Arc::clone(&storage) as Arc<dyn PersistenceService>);
        let gateway = Arc::new(gateway);
        info!("Gateway initialized");

        Self::init_feishu_adapter(config_dir, &gateway).await?;

        let shutdown = shutdown::ShutdownHandle::new();
        info!("Shutdown coordinator initialized");

        let llm_registry = Self::init_llm_registry().await;
        let chat_server = Self::spawn_chat_server(&llm_registry, &shutdown);
        let audit_logger = Self::spawn_audit_tasks(config_dir);

        info!(
            "CloseClaw daemon started successfully (v{})",
            env!("CARGO_PKG_VERSION")
        );

        Ok(Self {
            gateway,
            agent_registry,
            permission_engine,
            shutdown,
            chat_server,
            audit_logger,
            storage,
            sweeper_shutdown_tx: sweeper_tx,
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
        let _ = self.sweeper_shutdown_tx.send(());
        self.shutdown_audit().await;
        Ok(())
    }

    /// Run the daemon on non-Unix platforms (falls back to Ctrl+C only).
    #[cfg(not(unix))]
    pub async fn run(&self) -> anyhow::Result<()> {
        tokio::signal::ctrl_c().await?;
        info!("Received Ctrl+C, initiating shutdown...");
        self.shutdown.initiate_shutdown().await;
        let _ = self.sweeper_shutdown_tx.send(());
        self.shutdown_audit().await;
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

    /// Load and validate agents.json
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
            version: "1.0.0".to_string(),
            rules: Vec::new(),
            defaults: Defaults::default(),
            template_includes: Vec::new(),
            agent_creators: std::collections::HashMap::new(),
        };
        let mut engine = PermissionEngine::new(rule_set);

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

    /// Initialize LLM registry and register providers from environment variables.
    async fn init_llm_registry() -> Arc<LLMRegistry> {
        let registry = Arc::new(LLMRegistry::new());

        if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
            if !api_key.is_empty() {
                let provider = Arc::new(OpenAIProvider::new(api_key));
                registry.register("openai".to_string(), provider).await;
                info!("OpenAI provider registered");
            }
        }
        if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
            if !api_key.is_empty() {
                let provider = Arc::new(AnthropicProvider::new(api_key));
                registry.register("anthropic".to_string(), provider).await;
                info!("Anthropic provider registered");
            }
        }
        if let Ok(api_key) = std::env::var("MINIMAX_API_KEY") {
            if !api_key.is_empty() {
                let provider = Arc::new(MiniMaxProvider::new(api_key));
                registry.register("minimax".to_string(), provider).await;
                info!("MiniMax provider registered");
            }
        }

        registry
    }
}

impl Daemon {
    /// Create and spawn the chat TCP server as a background task.
    fn spawn_chat_server(
        llm_registry: &Arc<LLMRegistry>,
        shutdown: &shutdown::ShutdownHandle,
    ) -> Arc<ChatServer> {
        let chat_server = Arc::new(ChatServer::new(Arc::clone(llm_registry)));
        let chat_server_for_task = Arc::clone(&chat_server);
        let shutdown_rx = shutdown.subscribe_drain();
        tokio::spawn(async move {
            if let Err(e) = chat_server_for_task.run(shutdown_rx).await {
                tracing::warn!(error = %e, "chat server exited with error");
            }
        });
        info!(addr = "127.0.0.1:18889", "chat TCP server spawned");
        chat_server
    }

    /// Create audit logger and spawn background flush + startup-log tasks.
    fn spawn_audit_tasks(config_dir: &str) -> Arc<AuditLogger> {
        let audit_logger = Arc::new(AuditLogger::new());

        // Log daemon start as a config reload event
        let start_event = AuditEventBuilder::new(AuditEventType::ConfigReload)
            .details(serde_json::json!({
                "component": "daemon",
                "version": env!("CARGO_PKG_VERSION"),
                "config_dir": config_dir,
            }))
            .result(AuditResult::Allow)
            .build();
        let logger_for_start = Arc::clone(&audit_logger);
        tokio::spawn(async move {
            logger_for_start.log(start_event).await;
            logger_for_start.flush().await;
        });

        // Spawn background flush task
        let audit_for_flush = Arc::clone(&audit_logger);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        audit_for_flush.rotate_if_needed().await;
                        audit_for_flush.flush().await;
                    }
                }
            }
        });

        audit_logger
    }
}

// --- Audit & permission helpers ---

impl Daemon {
    /// Evaluate a permission request and log the result to the audit log
    pub async fn evaluate_with_audit(
        &self,
        request: crate::permission::PermissionRequest,
    ) -> crate::permission::PermissionResponse {
        let caller = request.caller().clone();
        let agent_id = caller.agent.clone();
        let response = self.permission_engine.evaluate(request.clone());

        let result = match &response {
            crate::permission::PermissionResponse::Allowed { .. } => AuditResult::Allow,
            crate::permission::PermissionResponse::Denied { .. } => AuditResult::Deny,
        };

        let event = AuditEventBuilder::new(AuditEventType::PermissionCheck)
            .details(serde_json::json!({
                "agent": agent_id,
                "user_id": caller.user_id,
                "request": request.body(),
            }))
            .result(result)
            .build();

        let logger = Arc::clone(&self.audit_logger);
        tokio::spawn(async move {
            logger.log(event).await;
        });

        response
    }

    /// Log an agent start event
    pub async fn log_agent_start(&self, agent_id: &str, model: &str) {
        let event = AuditEventBuilder::new(AuditEventType::AgentStart)
            .details(serde_json::json!({ "agent": agent_id, "model": model }))
            .result(AuditResult::Allow)
            .build();
        let logger = Arc::clone(&self.audit_logger);
        tokio::spawn(async move {
            logger.log(event).await;
        });
    }

    /// Log an agent stop event
    pub async fn log_agent_stop(&self, agent_id: &str) {
        let event = AuditEventBuilder::new(AuditEventType::AgentStop)
            .details(serde_json::json!({ "agent": agent_id }))
            .result(AuditResult::Allow)
            .build();
        let logger = Arc::clone(&self.audit_logger);
        tokio::spawn(async move {
            logger.log(event).await;
        });
    }

    /// Log an agent error event
    pub async fn log_agent_error(&self, agent_id: &str, error: &str) {
        let event = AuditEventBuilder::new(AuditEventType::AgentError)
            .details(serde_json::json!({ "agent": agent_id, "error": error }))
            .result(AuditResult::Error)
            .build();
        let logger = Arc::clone(&self.audit_logger);
        tokio::spawn(async move {
            logger.log(event).await;
        });
    }

    /// Shutdown the audit logger (flush remaining events)
    pub async fn shutdown_audit(&self) {
        self.audit_logger.shutdown().await;
    }
}

#[cfg(test)]
mod tests;
