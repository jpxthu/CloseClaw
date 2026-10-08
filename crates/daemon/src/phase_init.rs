//! Phase 1-3 daemon initialization: foundation, registries, core services.
//!
//! Extracted from `mod.rs` to keep source files within the CONTRIBUTING.md
//! limits (`mod.rs` only holds `pub use` / `pub mod` re-exports).

use super::Daemon;
use crate::metrics::NoopMetricsEmitter;
use crate::{llm_init, registries, shutdown, skill_reload, skills_helper};
use closeclaw_config::providers::SystemConfigData;
use closeclaw_config::session::SessionConfigProvider;
use closeclaw_config::{ConfigManager, ConfigSection};
use closeclaw_gateway::{Gateway, GatewayConfig, SessionManager};
use closeclaw_permission::PermissionEngine;
use closeclaw_session::{
    checkpoint_manager::CheckpointManager, persistence::PersistenceService, storage::SqliteStorage,
};
use closeclaw_skills::DiskSkillRegistry;
use closeclaw_system_prompt::sections::SectionCache;
use closeclaw_tools::ToolRegistry;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::sync::watch;
use tracing::{info, warn};

impl Daemon {
    /// Phase 1: Foundation — ConfigManager + Storage.
    pub(crate) fn init_phase_1_foundation(
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
    /// Phase 2: Registries — AgentRegistry, SkillsRegistry, ToolsRegistry,
    /// LLMRegistry, PermissionEngine, PlanArchiveSweeper.
    pub(crate) async fn init_phase_2_registries(
        config_dir: &str,
        config_manager: &ConfigManager,
        audit_logger: &Option<Arc<dyn closeclaw_common::AuditLogger>>,
    ) -> anyhow::Result<(
        Arc<closeclaw_agent::registry::AgentRegistry>,
        Arc<RwLock<Option<DiskSkillRegistry>>>,
        Arc<ToolRegistry>,
        Arc<RwLock<SectionCache>>,
        Arc<dyn SessionConfigProvider>,
        Arc<closeclaw_llm::LLMRegistry>,
        Arc<closeclaw_llm::unified_fallback::UnifiedFallbackClient>,
        Arc<tokio::sync::RwLock<PermissionEngine>>,
        tokio::sync::watch::Sender<()>,
        tokio::task::JoinHandle<()>,
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
                warn!("session config provider not available after load, using defaults");
                Arc::new(
                    closeclaw_config::session::JsonSessionConfigProvider::new("/dev/null").unwrap(),
                )
            });
        let data_dir = std::path::PathBuf::from(config_dir);
        let (plan_archive_shutdown_tx, plan_archive_sweeper_handle) =
            registries::spawn_plan_archive_sweeper(config_manager, &data_dir);
        // Parallel async components: skill_registry and llm_registry are
        // independent within Layer 2, so run them concurrently.
        let extra_dirs = skills_helper::resolve_extra_dirs(config_manager);
        let skill_fut = skill_reload::init_skill_registry(config_dir, None, extra_dirs);
        let llm_fut = Self::init_llm_registry(config_manager, llm_init::process_env);
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
            plan_archive_shutdown_tx,
            plan_archive_sweeper_handle,
        ))
    }
    /// Phase 3: Core services — Gateway, SessionManager, IM plugins, SlashDispatcher.
    pub(crate) async fn init_phase_3_core_services(
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
        Option<closeclaw_tasks::media_cleanup::MediaCleanupHandle>,
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
        // The processor chain is assembled here (composition root) and
        // injected as a common `ProcessorChain` trait object — the Gateway
        // never builds concrete processors itself.
        let processor_chain = crate::processor_registry::build_processor_chain(&gateway_config);
        // Same for the simplified-path outbound raw-log writer.
        let outbound_raw_log =
            crate::outbound_raw_log::build_outbound_raw_log_writer(&gateway_config);
        let gateway = Gateway::new(
            gateway_config,
            Arc::clone(&session_manager),
            processor_chain,
            outbound_raw_log,
        )
        .with_checkpoint_manager(Arc::clone(&checkpoint_manager));
        // Storage injection is now handled via the shared CheckpointManager
        // set on both SessionManager and Gateway above. The old
        // gateway.set_storage() path still works as a backward-compatible
        // wrapper that creates its own CheckpointManager internally.
        // Run session recovery scan: load all active checkpoints, detect
        // pending_operations, and persist recovery notifications/failure
        // results into checkpoints so resolve.rs can inject them when
        // sessions are restored.
        let (dirty_sessions_for_drain, migrated_sessions): (Vec<String>, Vec<String>) = {
            use closeclaw_session::recovery::SessionRecoveryService;
            let recovery_svc =
                SessionRecoveryService::new(Arc::clone(storage) as Arc<dyn PersistenceService>);
            // Inject the production workflow port so recovery scan can
            // re-inject workflow state (goal/jump messages) for sessions
            // with an active workflow run.
            recovery_svc
                .set_workflow_port(crate::workflow_port_adapter::engine_workflow_port())
                .await;
            let recovery_result =
                tokio::time::timeout(std::time::Duration::from_secs(10), recovery_svc.recover())
                    .await;
            match recovery_result {
                Ok(Ok(report)) => {
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
                    (report.dirty_sessions, report.migrated_sessions)
                }
                Ok(Err(e)) => {
                    warn!(error = %e, "recovery scan failed — continuing without recovery");
                    (Vec::new(), Vec::new())
                }
                Err(_) => {
                    warn!("recovery scan timed out (10s) — continuing without recovery");
                    (Vec::new(), Vec::new())
                }
            }
        };
        for sid in &migrated_sessions {
            session_manager.remove_stale_key_registry_entries(sid).await;
        }
        if let Err(e) = session_manager.rebuild_key_registry().await {
            warn!(error = %e, "failed to rebuild key_registry — continuing");
        }
        // Startup consistency check: SQLite ↔ file system bidirectional scan.
        if let Err(e) = session_manager.run_consistency_check().await {
            warn!(error = %e, "consistency check failed — continuing");
        }
        // Mark the scan timestamp so subsequent periodic checks are incremental.
        session_manager.initialize_consistency_check_time();
        if let Err(e) = session_manager.rebuild_spawn_tree().await {
            warn!(error = %e, "failed to rebuild spawn_tree — continuing");
        }
        let gateway = Arc::new(gateway);
        gateway
            .set_permission_engine(Arc::clone(permission_engine))
            .await;
        gateway.set_self_ref(Arc::clone(&gateway));
        // Wire Gateway back-reference into SessionManager (outbound pipeline).
        session_manager.set_gateway_ref(Arc::clone(&gateway)).await;
        gateway
            .set_config_dir(std::path::PathBuf::from(config_dir))
            .await;
        gateway
            .set_metrics_emitter(Arc::new(NoopMetricsEmitter))
            .await;
        let media_config_path = std::path::Path::new(config_dir).join("config/media.json");
        let media_config = match closeclaw_config::MediaConfigData::from_file(&media_config_path) {
            Ok(cfg) => cfg,
            Err(e) => {
                warn!(error = %e, path = %media_config_path.display(),
                    "failed to load media.json — using defaults");
                closeclaw_config::MediaConfigData::default()
            }
        };
        let shared_media_store =
            match closeclaw_im_adapter::media_store::MediaStore::new(&media_config.storage_dir) {
                Ok(store) => {
                    let store = Arc::new(store);
                    gateway.set_media_store(
                        store.clone() as Arc<dyn closeclaw_common::MediaStoreAccess>
                    );
                    gateway.set_media_config(media_config.clone());
                    Some(store)
                }
                Err(e) => {
                    warn!(error = %e, storage_dir = %media_config.storage_dir,
                        "failed to create MediaStore — media features disabled");
                    None
                }
            };
        closeclaw_im_adapter::platforms::register_platform_plugins(
            &gateway,
            config_dir,
            shared_media_store.clone(),
            Some(media_config.clone()),
        )
        .await;
        // Start the periodic media cleanup task.
        let media_cleanup_handle = if let Some(ref store) = shared_media_store {
            let retention_days = media_config.retention_days;
            let store_for_cleanup = Arc::new(store.as_ref().clone());
            let cleanup_fn: closeclaw_tasks::media_cleanup::CleanupFn = Arc::new(move |d| {
                store_for_cleanup
                    .cleanup_expired(d)
                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })
            });
            let rp: closeclaw_tasks::media_cleanup::RetentionProvider =
                Arc::new(move || retention_days);
            let handle = closeclaw_tasks::media_cleanup::start_media_cleanup(
                std::time::Duration::from_secs(3600),
                rp,
                cleanup_fn,
            );
            tracing::info!(retention_days, "media cleanup task started");
            Some(handle)
        } else {
            None
        };
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
                            warn!(
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
        let slash_registry = Self::init_slash_dispatcher(&gateway, &session_manager).await;
        // Start the inbound queue consumer so webhook messages are buffered.
        gateway.start_inbound_queue();
        // Read drain timeout from config; fall back to 30s default.
        let drain_timeout = config_manager
            .section(ConfigSection::System)
            .and_then(|v| serde_json::from_value::<SystemConfigData>(v).ok())
            .map(|sys| sys.effective_shutdown().drain_timeout_secs)
            .unwrap_or(30);
        let shutdown = shutdown::ShutdownHandle::new()
            .with_drain_timeout(std::time::Duration::from_secs(drain_timeout));
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
            media_cleanup_handle,
        ))
    }
}
/// Bundled shutdown receivers for background services.
/// Groups watch::Receiver<()> args to satisfy clippy's too_many_arguments.
pub(crate) struct ServiceShutdownReceivers {
    /// Receiver for ArchiveSweeper shutdown signal.
    pub sweeper: watch::Receiver<()>,
    /// Receiver for AnnounceSweeper shutdown signal.
    pub announce_sweeper: watch::Receiver<()>,
    /// Receiver for DreamingScheduler shutdown signal.
    pub dreaming: watch::Receiver<()>,
}
