//! Daemon - CloseClaw background service
//!
//! Orchestrates all components: Gateway, AgentRegistry, PermissionEngine.
//! Handles graceful shutdown via ShutdownCoordinator.

pub mod shutdown;

use crate::audit::{AuditEvent, AuditEventBuilder, AuditLogger, AuditResult, AuditEventType};
use crate::chat::ChatServer;
use crate::config::agents::AgentsConfigProvider;
use crate::config::providers::ConfigProvider;
use crate::gateway::{Gateway, GatewayConfig};
use crate::im::feishu::FeishuAdapter;
use crate::permission::{Defaults, PermissionEngine, RuleSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, error};

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
}

impl Daemon {
    /// Start the daemon with the given config directory
    pub async fn start(config_dir: &str) -> anyhow::Result<Self> {
        info!("Starting CloseClaw daemon with config_dir={}", config_dir);

        // Load agents config
        let agents_config = Self::load_agents_config(config_dir)?;

        // Initialize permission engine (rule evaluation sandbox)
        let rule_set = RuleSet {
            version: "1.0.0".to_string(),
            rules: Vec::new(),
            defaults: Defaults::default(),
            template_includes: Vec::new(),
            agent_creators: std::collections::HashMap::new(),
        };
        let permission_engine = Arc::new(PermissionEngine::new(rule_set));
        info!("Permission engine initialized");

        // Initialize agent registry
        let agent_registry: Arc<RwLock<crate::agent::registry::AgentRegistry>> =
            Arc::new(RwLock::new(crate::agent::registry::AgentRegistry::new(30)));
        info!("Agent registry initialized ({} agents)", agents_config.agents().len());

        // Initialize gateway
        let gateway = Arc::new(Gateway::new(GatewayConfig {
            name: "closeclaw".to_string(),
            rate_limit_per_minute: 60,
            max_message_size: 16_384,
        }));
        info!("Gateway initialized");

        // Register Feishu adapter if credentials are available
        Self::init_feishu_adapter(config_dir, &gateway).await?;

        // Initialize shutdown coordinator
        let shutdown = shutdown::ShutdownHandle::new();
        info!("Shutdown coordinator initialized");

        // Initialize and spawn chat TCP server
        let chat_server = Arc::new(ChatServer::new());
        let chat_server_for_task = Arc::clone(&chat_server);
        let shutdown_rx = shutdown.subscribe_drain();
        tokio::spawn(async move {
            if let Err(e) = chat_server_for_task.run(shutdown_rx).await {
                tracing::warn!(error = %e, "chat server exited with error");
            }
        });
        info!(addr = "127.0.0.1:18889", "chat TCP server spawned");

        // Initialize audit logger
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
        let audit_logger_for_start = Arc::clone(&audit_logger);
        tokio::spawn(async move {
            audit_logger_for_start.log(start_event).await;
            audit_logger_for_start.flush().await;
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
        })
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

    /// Initialize Feishu adapter from env or config
    async fn init_feishu_adapter(
        _config_dir: &str,
        gateway: &Arc<Gateway>,
    ) -> anyhow::Result<()> {
        let app_id = std::env::var("FEISHU_APP_ID").ok();
        let app_secret = std::env::var("FEISHU_APP_SECRET").ok();
        let verification_token = std::env::var("FEISHU_VERIFICATION_TOKEN").ok();

        if let (Some(app_id), Some(app_secret), Some(verification_token)) =
            (app_id, app_secret, verification_token)
        {
            let adapter = Arc::new(FeishuAdapter::new(
                app_id,
                app_secret,
                verification_token,
            ));
            gateway
                .register_adapter("feishu".to_string(), adapter)
                .await;
            info!("Feishu adapter registered");
        } else {
            info!("Feishu credentials not found in env — Feishu adapter not registered");
        }
        Ok(())
    }

    /// Run the daemon — blocks until shutdown
    pub async fn run(&self) -> anyhow::Result<()> {
        // Block forever — all work is async via tokio
        tokio::signal::ctrl_c().await?;
        info!("Received Ctrl+C, initiating shutdown...");
        self.shutdown.initiate_shutdown().await;
        // Flush audit logs before exit
        self.shutdown_audit().await;
        Ok(())
    }

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
            .details(serde_json::json!({
                "agent": agent_id,
                "model": model,
            }))
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
            .details(serde_json::json!({
                "agent": agent_id,
            }))
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
            .details(serde_json::json!({
                "agent": agent_id,
                "error": error,
            }))
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
