//! Daemon lifecycle: start, run, and shutdown phases.

use super::{Daemon, Phase5Deps};
use closeclaw_permission::{Defaults, PermissionEngine, RuleSet};
use std::sync::Arc;
use tracing::info;

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
        permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
    ) -> anyhow::Result<Self> {
        info!("Starting CloseClaw daemon with config_dir={}", config_dir);
        Self::load_env(config_dir);
        let (startup_layers, _phase_components) = Self::resolve_startup_order()?;
        Self::log_startup_order(&startup_layers);
        let (config_manager, storage, data_dir) = Self::init_phase_1_foundation(config_dir)?;
        let (agent_registry, skill_registry, tool_registry, skill_watcher) =
            Self::init_phase_2_registries(config_dir).await?;
        let (gateway, session_manager, shutdown) = Self::init_phase_3_core_services(
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
        let approval_flow = Self::init_phase_4_wiring(
            &gateway,
            &session_manager,
            &permission_engine,
            &config_manager,
            config_dir,
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
        ) = Self::init_phase_5_background(
            Phase5Deps {
                config_manager: &config_manager,
                agent_registry: &agent_registry,
                skill_registry: &skill_registry,
                tool_registry: &tool_registry,
                session_manager: &session_manager,
                permission_engine: &permission_engine,
                approval_flow: &approval_flow,
                gateway: &gateway,
            },
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
            announce_shutdown_tx: announce_sweeper_tx,
            dreaming_scheduler_shutdown_tx: dreaming_tx,
            plan_archive_shutdown_tx: plan_archive_tx,
            skill_registry,
            _skill_watcher: Some(skill_watcher),
            _config_watcher: config_watcher,
            approval_flow,
            admin_handle: Some(admin_handle),
            admin_socket_path: admin_sock_path,
            archive_sweeper_handle: Some(sweeper_handle),
            announce_sweeper_handle: Some(announce_sweeper_handle),
            dreaming_scheduler_handle: Some(dreaming_handle),
            plan_archive_task_handle: Some(plan_archive_handle),
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

        // Wait for all background tasks to exit (15s timeout per task)
        let join_timeout = std::time::Duration::from_secs(10);

        if let Some(handle) = self.archive_sweeper_handle.take() {
            match tokio::time::timeout(join_timeout, handle).await {
                Ok(Ok(())) => tracing::info!("ArchiveSweeper exited cleanly"),
                Ok(Err(e)) => tracing::warn!(error = %e, "ArchiveSweeper task panicked"),
                Err(_) => tracing::warn!("ArchiveSweeper did not exit within 10s, continuing"),
            }
        }

        if let Some(handle) = self.announce_sweeper_handle.take() {
            match tokio::time::timeout(join_timeout, handle).await {
                Ok(Ok(())) => tracing::info!("AnnounceSweeper exited cleanly"),
                Ok(Err(e)) => tracing::warn!(error = %e, "AnnounceSweeper task panicked"),
                Err(_) => tracing::warn!("AnnounceSweeper did not exit within 10s, continuing"),
            }
        }

        if let Some(handle) = self.dreaming_scheduler_handle.take() {
            match tokio::time::timeout(join_timeout, handle).await {
                Ok(Ok(())) => tracing::info!("DreamingScheduler exited cleanly"),
                Ok(Err(e)) => tracing::warn!(error = %e, "DreamingScheduler task panicked"),
                Err(_) => tracing::warn!("DreamingScheduler did not exit within 10s, continuing"),
            }
        }

        if let Some(handle) = self.plan_archive_task_handle.take() {
            match tokio::time::timeout(join_timeout, handle).await {
                Ok(Ok(())) => tracing::info!("PlanArchiveTask exited cleanly"),
                Ok(Err(e)) => tracing::warn!(error = %e, "PlanArchiveTask task panicked"),
                Err(_) => tracing::warn!("PlanArchiveTask did not exit within 10s, continuing"),
            }
        }

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
    pub(crate) fn build_permission_engine(
        config_dir: &str,
    ) -> Arc<tokio::sync::RwLock<PermissionEngine>> {
        let rule_set = RuleSet {
            rules: Vec::new(),
            defaults: Defaults::default(),
            user_defaults: Defaults::user_defaults(),
            template_includes: Vec::new(),
            agent_creators: std::collections::HashMap::new(),
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
        info!("Permission engine initialized");
        Arc::new(tokio::sync::RwLock::new(engine))
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
    pub(crate) async fn init_slash_dispatcher(
        gateway: &Arc<closeclaw_gateway::Gateway>,
        session_manager: &Arc<closeclaw_gateway::SessionManager>,
        permission_engine: &Arc<tokio::sync::RwLock<PermissionEngine>>,
    ) {
        use closeclaw_slash::dispatcher::SlashDispatcher;
        use closeclaw_slash::handlers::{ReasoningHandler, SystemHandler, WorkdirHandler};
        use closeclaw_slash::handlers_bg::BackgroundHandler;
        use closeclaw_slash::handlers_permission::PermissionSlashHandler;
        use closeclaw_slash::handlers_user::UserSlashHandler;
        use closeclaw_slash::registry::HandlerRegistry;
        use closeclaw_slash::{
            ClearHandler, CompactHandler, ExecHandler, ExecuteHandler, HelpHandler, ModeHandler,
            NewSessionHandler, PauseHandler, PlanModeHandler, StatusHandler, StopHandler,
            VerboseHandler,
        };

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
        slash_registry.register(Arc::new(PlanModeHandler::new(Arc::clone(session_manager))));
        slash_registry.register(Arc::new(ModeHandler::new(Arc::clone(session_manager))));
        slash_registry.register(Arc::new(ExecuteHandler::new(Arc::clone(session_manager))));
        slash_registry.register(Arc::new(PauseHandler::new(Arc::clone(session_manager))));
        slash_registry.register(Arc::new(BackgroundHandler::new(Arc::clone(
            session_manager,
        ))));
        slash_registry.register(Arc::new(PermissionSlashHandler));
        if let Some(config_dir) = gateway.get_config_dir().await {
            slash_registry.register(Arc::new(UserSlashHandler::new(config_dir)));
        }
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
