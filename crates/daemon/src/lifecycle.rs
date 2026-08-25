//! Daemon lifecycle: start, run, and shutdown phases.

use super::{Daemon, Phase5Deps};
use closeclaw_config::SystemConfigData;
use closeclaw_debug_log::{DebugLog, DebugLogConfig};
use closeclaw_permission::engine::audit_log::{AuditLogger, FileAuditLogger};
use closeclaw_permission::engine::rejection_log::FileRejectionLogger;
use closeclaw_permission::{Defaults, PermissionEngine, RuleSet};
use std::sync::Arc;
use tracing::{error, info, warn};

pub(crate) use crate::llm_components::assemble_llm_components;

impl Daemon {
    /// Start the daemon with the given config directory.
    pub async fn start(config_dir: &str) -> anyhow::Result<Self> {
        let audit_logger = Self::create_audit_logger(config_dir);
        Self::start_with_engine(config_dir, audit_logger).await
    }
    /// Start the daemon with an optional audit logger.
    ///
    /// The `audit_logger` is injected into the [`PermissionEngine`] built
    /// during phase-2 initialization. If `None`, the engine runs without
    /// audit logging.
    pub async fn start_with_engine(
        config_dir: &str,
        audit_logger: Option<Arc<dyn AuditLogger>>,
    ) -> anyhow::Result<Self> {
        info!("Starting CloseClaw daemon with config_dir={}", config_dir);
        Self::load_env(config_dir);
        let (startup_layers, _phase_components) = Self::resolve_startup_order()?;
        Self::log_startup_order(&startup_layers);
        let (config_manager, storage, data_dir) = Self::init_phase_1_foundation(config_dir)?;
        let (
            agent_registry,
            skill_registry,
            tool_registry,
            skill_watcher,
            shared_cache,
            session_config_provider,
            llm_registry,
            skill_rescan_handle,
            permission_engine,
        ) = Self::init_phase_2_registries(config_dir, &config_manager, &audit_logger).await?;
        let (gateway, session_manager, shutdown, dirty_sessions, slash_registry) =
            Self::init_phase_3_core_services(
                config_dir,
                &storage,
                &permission_engine,
                &config_manager,
            )
            .await?;
        let shutdown = Arc::new(shutdown);
        // Wire shutdown handle into Gateway and SessionManager for
        // busy-count tracking during drain.
        let common_sh = crate::bridge::common_shutdown_handle(&shutdown);
        gateway.set_shutdown_handle(Arc::clone(&common_sh));
        session_manager.set_shutdown_handle(common_sh).await;
        // Inject the independently created SessionConfigProvider into
        // SessionManager so per-agent idle/purge thresholds resolve
        // without going through ConfigManager.
        session_manager
            .set_session_config_provider(session_config_provider.clone())
            .await;
        // Initialize DebugLog from config and inject into Gateway.
        // Config missing or invalid → Gateway runs without debug logging.
        if let Some(debug_log) = Self::init_debug_log(config_dir).await {
            gateway.set_debug_log(debug_log).await;
            info!("DebugLog injected into Gateway");
        }
        let (approval_flow, builtin_skill_registry) = Self::init_phase_4_wiring(
            &gateway,
            &session_manager,
            &permission_engine,
            &config_manager,
            config_dir,
            audit_logger,
        )
        .await;
        let (
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
            system_prompt_builder,
        ) = Self::init_phase_5_background(
            Phase5Deps {
                config_manager: &config_manager,
                agent_registry: &agent_registry,
                skill_registry: &skill_registry,
                builtin_skill_registry: &builtin_skill_registry,
                tool_registry: &tool_registry,
                session_manager: &session_manager,
                permission_engine: &permission_engine,
                approval_flow: &approval_flow,
                gateway: &gateway,
                slash_registry: &slash_registry,
                shared_cache: &shared_cache,
            },
            &data_dir,
            Arc::clone(&session_config_provider),
        )
        .await?;

        // LLM call chain assembly: CacheAdapter → UnifiedChatClient → FallbackChain
        // → LLMCaller. Must happen after Phase 5.
        let provider_ids = llm_registry.list().await;
        let mut chain_entries: Vec<closeclaw_llm::unified_fallback::ChainEntry> = Vec::new();
        for provider_id in &provider_ids {
            if let Some(provider) = llm_registry.get(provider_id).await {
                let cache_adapter = closeclaw_llm::cache_adapter::for_provider(provider_id);

                // Per-provider assembly: protocol / interpreter / plugin (design doc llm/README.md)
                let (protocol, interpreter_registry, plugin_pipeline) =
                    assemble_llm_components(provider_id.as_str());

                let client = closeclaw_llm::UnifiedChatClient::new(
                    provider,
                    protocol,
                    interpreter_registry,
                    plugin_pipeline,
                    cache_adapter,
                );
                chain_entries.push(closeclaw_llm::unified_fallback::ChainEntry {
                    provider_id: provider_id.clone(),
                    model_id: provider_id.clone(),
                    client: Arc::new(client),
                });
            }
        }
        let cooldown_manager = Arc::new(closeclaw_llm::retry::CooldownManager::new());
        let unified_fallback =
            Arc::new(closeclaw_llm::unified_fallback::UnifiedFallbackClient::new(
                chain_entries,
                Arc::clone(&cooldown_manager),
            ));
        let fallback_llm_caller = Arc::new(closeclaw_gateway::llm_caller_impl::FallbackLlmCaller(
            Arc::clone(&unified_fallback),
        ));
        session_manager
            .set_llm_caller(fallback_llm_caller as Arc<dyn closeclaw_common::LlmCaller>)
            .await;
        info!(count = provider_ids.len(), "LLM call chain assembled");

        // Create SessionMessageHandler for busy/pending state machine.
        let (output_tx, output_rx) = tokio::sync::mpsc::channel(64);
        let active_searcher_llm_caller = Arc::new(
            closeclaw_gateway::session_handler::ActiveSearcherLlmCaller {
                client: Arc::clone(&unified_fallback),
                model: String::new(),
            },
        );
        #[allow(deprecated)]
        let fallback_client_for_compact =
            Arc::new(closeclaw_llm::fallback::FallbackClient::from_strings(
                Arc::clone(&llm_registry),
                vec![],
            ));
        let session_handler = Arc::new(closeclaw_gateway::SessionMessageHandler::new(
            Arc::clone(&session_manager),
            fallback_client_for_compact,
            output_tx,
            active_searcher_llm_caller,
            closeclaw_common::CompactConfig::default(),
        ));
        gateway.set_session_handler(session_handler);

        // Inject recovery notifications into dirty sessions at startup.
        // Must happen after Phase 5 when LLM caller, system prompt builder,
        // and other dependencies are wired into SessionManager.
        session_manager
            .inject_startup_recovery_notifications(&dirty_sessions)
            .await;
        // Recovery injection may have created new ConversationSession / Session
        // entries. Rebuild key_registry so they are resolvable by routing key.
        if let Err(e) = session_manager.rebuild_key_registry().await {
            tracing::warn!(
                error = %e,
                "failed to rebuild key_registry after recovery injection \
                 — continuing"
            );
        }
        let (admin_handle, admin_sock_path) = Self::init_phase_6_admin_rpc(
            &agent_registry,
            &skill_registry,
            &config_manager,
            config_dir,
            skill_rescan_handle,
        )
        .await;
        let (chat_handle, chat_sock_path) = Self::init_phase_6_chat_rpc(&gateway, config_dir).await;
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
            announce_shutdown_tx: announce_sweeper_tx,
            dreaming_scheduler_shutdown_tx: dreaming_tx,
            plan_archive_shutdown_tx: plan_archive_tx,
            skill_registry,
            builtin_skill_registry,
            slash_registry,
            _skill_watcher: skill_watcher,
            _config_watcher: config_watcher,
            approval_flow,
            admin_handle: Some(admin_handle),
            admin_socket_path: admin_sock_path,
            chat_handle: Some(chat_handle),
            chat_socket_path: chat_sock_path,
            archive_sweeper_handle: Some(sweeper_handle),
            announce_sweeper_handle: Some(announce_sweeper_handle),
            dreaming_scheduler_handle: Some(dreaming_handle),
            plan_archive_task_handle: Some(plan_archive_handle),
            spawn_controller: Some(spawn_controller),
            system_prompt_builder: Some(system_prompt_builder),
            llm_registry: Arc::clone(&llm_registry),
            _output_rx: output_rx,
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

        // Phase 0: Send brief start notification (no session details yet)
        self.gateway
            .send_shutdown_start_notification(self.shutdown.mode())
            .await;

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
        let mut shutdown_task =
            tokio::spawn(async move { shutdown_handle.initiate_shutdown().await });

        // Monitor for escalation signals during drain
        loop {
            tokio::select! {
                result = &mut shutdown_task => {
                    match result {
                        Ok(remaining) => {
                            if remaining > 0 {
                                info!(
                                    remaining,
                                    "drain completed with {} operations still in-flight",
                                    remaining
                                );
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "shutdown task panicked");
                        }
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
        let timeout = closeclaw_session::llm_session::session_handles::DEFAULT_GRACEFUL_TIMEOUT;
        let mut stop_handle = tokio::spawn(async move {
            sm.stop_all_sessions(mode, timeout, Some(&progress_tx))
                .await
        });

        // Spawn fresh signal handlers for escalation monitoring during Phase 2.
        // Phase 1's handlers are consumed by its tokio::select! loop.
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt()).ok();
        let mut sigterm = signal(SignalKind::terminate()).ok();

        // Heartbeat state: send every 30s when no progress events arrive.
        let heartbeat_interval = std::time::Duration::from_secs(30);
        let phase2_start = std::time::Instant::now();
        let mut last_event: tokio::time::Instant = tokio::time::Instant::now();

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
                            error!(error = %e, "session stop task panicked");
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
                    last_event = tokio::time::Instant::now();
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

                _ = tokio::time::sleep_until(
                    last_event + heartbeat_interval
                ) => {
                    // 30s with no events — send heartbeat notification
                    let current_mode: closeclaw_common::shutdown::ShutdownMode =
                        self.shutdown.mode();
                    let longest_wait_secs = phase2_start.elapsed().as_secs();
                    let active_count = {
                        let conv = self
                            .gateway
                            .session_manager()
                            .conversation_sessions
                            .read()
                            .await;
                        conv.values().filter(|cs| {
                            !cs.try_read().map_or(true, |c| c.is_stopped())
                        }).count()
                    };
                    tracing::info!(
                        active_count,
                        longest_wait_secs,
                        "Phase 2 heartbeat — sending periodic notification"
                    );
                    self.gateway
                        .send_shutdown_heartbeat_card(active_count, longest_wait_secs, current_mode)
                        .await;
                    // Reset heartbeat timer
                    last_event = tokio::time::Instant::now();
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
    /// - Verifies all background tasks have exited (abort + confirm)
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
        // Signal AnnounceSweeper to stop
        let _ = self.announce_shutdown_tx.send(());
        // Signal DreamingScheduler to stop
        let _ = self.dreaming_scheduler_shutdown_tx.send(());
        // Signal PlanArchiveTask to stop
        let _ = self.plan_archive_shutdown_tx.send(());

        // Wait for all background tasks to exit, aborting on timeout.
        let join_timeout = std::time::Duration::from_secs(10);
        let abort_grace = std::time::Duration::from_secs(3);

        if let Some(handle) = self.archive_sweeper_handle.take() {
            Self::abort_and_join_background_task(
                handle,
                "ArchiveSweeper",
                join_timeout,
                abort_grace,
            )
            .await;
        }

        if let Some(handle) = self.announce_sweeper_handle.take() {
            Self::abort_and_join_background_task(
                handle,
                "AnnounceSweeper",
                join_timeout,
                abort_grace,
            )
            .await;
        }

        if let Some(handle) = self.dreaming_scheduler_handle.take() {
            Self::abort_and_join_background_task(
                handle,
                "DreamingScheduler",
                join_timeout,
                abort_grace,
            )
            .await;
        }

        if let Some(handle) = self.plan_archive_task_handle.take() {
            Self::abort_and_join_background_task(
                handle,
                "PlanArchiveTask",
                join_timeout,
                abort_grace,
            )
            .await;
        }

        // Clear pending approval requests (denied with callbacks triggered)
        self.approval_flow.lock().await.clear();
    }

    /// Wait for a background task to exit within `timeout`.
    ///
    /// If the task does not exit in time, it is aborted and a short
    /// `abort_grace` is given to confirm termination.  A final log is
    /// emitted if the task is still alive after abort (theoretically
    /// impossible, but logged defensively at error level).
    async fn abort_and_join_background_task(
        mut handle: tokio::task::JoinHandle<()>,
        name: &str,
        timeout: std::time::Duration,
        abort_grace: std::time::Duration,
    ) {
        match tokio::time::timeout(timeout, &mut handle).await {
            Ok(Ok(())) => {
                info!("{} exited cleanly", name);
            }
            Ok(Err(e)) => {
                warn!(error = %e, "{} task panicked", name);
            }
            Err(_) => {
                warn!("{} did not exit within {:?}, aborting", name, timeout);
                handle.abort();
                match tokio::time::timeout(abort_grace, handle).await {
                    Ok(Ok(())) => {
                        info!("{} terminated after abort", name);
                    }
                    Ok(Err(_)) => {
                        info!("{} task panicked on abort join — terminated", name);
                    }
                    Err(_) => {
                        error!("{} still alive after abort — possible resource leak", name);
                    }
                }
            }
        }
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
        // Clean up chat socket file
        let _ = tokio::fs::remove_file(&self.chat_socket_path).await;
    }
}

// --- Config loading helpers ---
impl Daemon {
    /// Load .env file from config_dir if it exists.
    pub(crate) fn load_env(config_dir: &str) {
        let env_path = std::path::Path::new(config_dir).join(".env");
        if env_path.exists() {
            if let Err(e) = super::load_env_file(&env_path) {
                tracing::warn!(error = %e, path = %env_path.display(), "failed to load .env file");
            } else {
                info!("Loaded environment from {}", env_path.display());
            }
        }
    }

    /// Read BOOTSTRAP_MODE env var and convert to BootstrapMode.
    /// "minimal" → Minimal, anything else (including absent) → Full.
    #[allow(dead_code)]
    pub(crate) fn read_bootstrap_mode() -> closeclaw_session::bootstrap::BootstrapMode {
        match std::env::var("BOOTSTRAP_MODE").as_deref() {
            Ok("minimal") => closeclaw_session::bootstrap::BootstrapMode::Minimal,
            _ => closeclaw_session::bootstrap::BootstrapMode::Full,
        }
    }

    /// Build permission engine, loading templates from config_dir/templates/ if present.
    ///
    /// When a `rejection_log` section is present in `system.json`, a
    /// [`FileRejectionLogger`] with the configured `max_entries` limit is
    /// injected via [`PermissionEngine::with_rejection_logger`].
    pub(crate) fn build_permission_engine(
        config_dir: &str,
        audit_logger: Option<Arc<dyn AuditLogger>>,
    ) -> Arc<tokio::sync::RwLock<PermissionEngine>> {
        let rule_set = RuleSet {
            rules: Vec::new(),
            defaults: Defaults::default(),
            user_defaults: Defaults::user_defaults(),
            template_includes: Vec::new(),
            rule_version: String::new(),
        };
        let mut engine = PermissionEngine::new(rule_set, std::path::PathBuf::from(config_dir));
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
        // Inject rejection log logger if configured in system.json.
        let engine = Self::wire_rejection_logger(engine, config_dir);
        // Inject audit log logger if provided.
        let engine = if let Some(logger) = audit_logger {
            engine.with_audit_logger(logger)
        } else {
            engine
        };
        info!("Permission engine initialized");
        Arc::new(tokio::sync::RwLock::new(engine))
    }

    /// Read `rejection_log` config from `system.json` and inject the
    /// logger into the permission engine.
    fn wire_rejection_logger(mut engine: PermissionEngine, config_dir: &str) -> PermissionEngine {
        let system_path = std::path::Path::new(config_dir).join("system.json");
        if !system_path.exists() {
            return engine;
        }
        match SystemConfigData::from_file(&system_path) {
            Ok(sys_cfg) => {
                if let Some(rejection_cfg) = sys_cfg.rejection_log {
                    let log_path = std::path::Path::new(config_dir)
                        .join("logs")
                        .join("rejection.log");
                    match FileRejectionLogger::new_with_limit(log_path, rejection_cfg.max_entries) {
                        Ok(logger) => {
                            engine = engine.with_rejection_logger(Arc::new(logger));
                            info!(
                                max_entries = ?rejection_cfg.max_entries,
                                "Rejection log logger configured"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "Failed to create rejection log logger — continuing without"
                            );
                        }
                    }
                }
            }
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "system.json not found or invalid — skipping rejection log config"
                );
            }
        }
        engine
    }

    /// Read `audit_log` config from `system.json` and create a
    /// [`FileAuditLogger`] if configured.
    fn create_audit_logger(config_dir: &str) -> Option<Arc<dyn AuditLogger>> {
        let system_path = std::path::Path::new(config_dir).join("system.json");
        if !system_path.exists() {
            return None;
        }
        match SystemConfigData::from_file(&system_path) {
            Ok(sys_cfg) => {
                if let Some(audit_cfg) = sys_cfg.audit_log {
                    let log_path = std::path::Path::new(config_dir)
                        .join("logs")
                        .join("audit.log");
                    match FileAuditLogger::new_with_limit(log_path, audit_cfg.max_entries) {
                        Ok(logger) => {
                            info!(
                                max_entries = ?audit_cfg.max_entries,
                                "Audit log logger configured"
                            );
                            Some(Arc::new(logger))
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "Failed to create audit log logger — continuing without"
                            );
                            None
                        }
                    }
                } else {
                    None
                }
            }
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "system.json not found or invalid — skipping audit log config"
                );
                None
            }
        }
    }

    /// Migrate legacy openclaw.json if present (non-fatal on error).
    pub(crate) fn run_config_migration(config_dir: &str) {
        let openclaw_json_path = std::path::Path::new(config_dir).join("openclaw.json");
        info!("Checking for legacy openclaw.json migration...");
        match closeclaw_config::migration::migrate_if_needed(&openclaw_json_path, config_dir) {
            Ok(true) => info!("Legacy openclaw.json migration completed successfully"),
            Ok(false) => info!("No migration needed — config directory is up to date"),
            Err(e) => tracing::warn!(
                error = %e,
                "openclaw.json migration failed — continuing with existing config"
            ),
        }
    }

    /// Initialize the debug log framework from config.
    ///
    /// Reads `{config_dir}/config/debug_log.json`. If the file is missing
    /// or invalid, returns `None` — the daemon continues without debug logging.
    async fn init_debug_log(config_dir: &str) -> Option<DebugLog> {
        let config_path = std::path::Path::new(config_dir)
            .join("config")
            .join("debug_log.json");
        if !config_path.exists() {
            tracing::debug!("debug_log.json not found — skipping debug log init");
            return None;
        }
        match DebugLogConfig::from_file(&config_path).await {
            Ok(config) => match DebugLog::new(config).await {
                Ok(debug_log) => Some(debug_log),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "failed to create DebugLog instance — continuing without"
                    );
                    None
                }
            },
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %config_path.display(),
                    "failed to load debug_log.json — continuing without"
                );
                None
            }
        }
    }
}

// --- Service init helpers ---
impl Daemon {
    /// Initialize the terminal (CLI) IM plugin and register with Gateway.
    pub(crate) async fn init_terminal_plugin(gateway: &Arc<closeclaw_gateway::Gateway>) {
        use closeclaw_cli::terminal::TerminalPlugin;
        let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(TerminalPlugin::new());
        gateway.register_plugin(plugin).await;
        info!("Terminal plugin registered");
    }

    /// Initialize the slash command dispatcher and register all handlers.
    ///
    /// Returns the shared [`HandlerRegistry`] so callers can later register
    /// additional handlers (e.g. [`SkillSlashHandler`]) after dependent
    /// registries are initialized.
    pub(crate) async fn init_slash_dispatcher(
        gateway: &Arc<closeclaw_gateway::Gateway>,
        session_manager: &Arc<closeclaw_gateway::SessionManager>,
        permission_engine: &Arc<tokio::sync::RwLock<PermissionEngine>>,
    ) -> Arc<closeclaw_slash::registry::HandlerRegistry> {
        use closeclaw_slash::dispatcher::SlashDispatcher;
        use closeclaw_slash::handlers::{ReasoningHandler, SystemHandler, WorkdirHandler};
        use closeclaw_slash::handlers_bg::BackgroundHandler;
        use closeclaw_slash::handlers_permission::PermissionSlashHandler;
        use closeclaw_slash::handlers_user::UserSlashHandler;
        use closeclaw_slash::registry::HandlerRegistry;
        use closeclaw_slash::{
            AutoModeHandler, ClearHandler, CompactHandler, ExecHandler, ExecuteHandler,
            HelpHandler, ModeHandler, NewSessionHandler, PauseHandler, PlanBrowseHandler,
            PlanModeHandler, StatusHandler, StopHandler, VerboseHandler,
        };

        let sm_query: Arc<dyn closeclaw_common::SlashSessionQuery> = session_manager.clone();
        let slash_registry = Arc::new(HandlerRegistry::new());
        let registry_for_return = Arc::clone(&slash_registry);
        slash_registry.register(Arc::new(CompactHandler));
        slash_registry.register(Arc::new(ClearHandler::new(Arc::clone(&sm_query))));
        slash_registry.register(Arc::new(ExecHandler));
        slash_registry.register(Arc::new(WorkdirHandler::new(Arc::clone(&sm_query))));
        let help_handler = HelpHandler::new(Arc::clone(&slash_registry));
        slash_registry.register(Arc::new(help_handler));
        slash_registry.register(Arc::new(ReasoningHandler::new(Arc::clone(&sm_query))));
        slash_registry.register(Arc::new(VerboseHandler::new(Arc::clone(&sm_query))));
        slash_registry.register(Arc::new(SystemHandler::new(Arc::clone(&sm_query))));
        slash_registry.register(Arc::new(NewSessionHandler));
        slash_registry.register(Arc::new(StopHandler));
        slash_registry.register(Arc::new(StatusHandler::new(Arc::clone(&sm_query))));
        let plan_handler = Arc::new(PlanModeHandler::new(Arc::clone(&sm_query)));
        let auto_handler = Arc::new(AutoModeHandler::new(Arc::clone(&sm_query)));
        slash_registry.register(plan_handler.clone() as Arc<dyn closeclaw_common::SlashHandler>);
        slash_registry.register(Arc::new(ModeHandler::with_handlers(
            Arc::clone(&sm_query),
            plan_handler,
            auto_handler,
        )));
        slash_registry.register(Arc::new(ExecuteHandler::new(Arc::clone(&sm_query))));
        slash_registry.register(Arc::new(PauseHandler::new(Arc::clone(&sm_query))));
        slash_registry.register(Arc::new(BackgroundHandler::new(Arc::clone(&sm_query))));
        slash_registry.register(Arc::new(PlanBrowseHandler::new(Arc::clone(&sm_query))));
        slash_registry.register(Arc::new(PermissionSlashHandler));
        if let Some(config_dir) = gateway.get_config_dir().await {
            slash_registry.register(Arc::new(UserSlashHandler::new(config_dir)));
        }
        let slash_dispatcher = Arc::new(SlashDispatcher::from_shared(slash_registry))
            as Arc<dyn closeclaw_common::SlashRouter>;
        gateway.set_slash_dispatcher(slash_dispatcher).await;
        // 高危 slash 指令（如 /exec）需要权限引擎介入；在此注入使得
        // dispatch_slash 在 Branch 2 时能取到 engine。
        gateway
            .set_permission_engine(Arc::clone(permission_engine))
            .await;
        info!("Slash dispatcher installed");
        registry_for_return
    }
}
