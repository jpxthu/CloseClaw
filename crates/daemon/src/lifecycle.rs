//! Daemon lifecycle: start, run, and shutdown phases.

use super::{Daemon, Phase5Deps};
use closeclaw_debug_log::{DebugLog, DebugLogConfig};
use closeclaw_permission::engine::audit_log::AuditLogger;
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::shutdown_heartbeat::ShutdownHeartbeat;

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
            shared_cache,
            session_config_provider,
            llm_registry,
            fallback_client,
            permission_engine,
            plan_archive_shutdown_tx,
            plan_archive_sweeper_handle,
        ) = Self::init_phase_2_registries(config_dir, &config_manager, &audit_logger).await?;
        let (
            gateway,
            session_manager,
            shutdown,
            dirty_sessions,
            slash_registry,
            media_cleanup_handle,
        ) = Self::init_phase_3_core_services(
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

        // Create PlanExecConfirmFlow for independent plan-execution confirmation.
        let confirm_flow = {
            let sm_lookup: Arc<dyn closeclaw_common::SessionLookup> =
                Arc::clone(&session_manager) as Arc<dyn closeclaw_common::SessionLookup>;
            let on_notify: Arc<
                dyn Fn(closeclaw_tools::builtin::PlanExecNotification) + Send + Sync,
            > = Arc::new(|_| {});
            Arc::new(closeclaw_tools::builtin::PlanExecConfirmFlow::new(
                sm_lookup,
                on_notify,
                tokio::runtime::Handle::current(),
            ))
        };
        let (
            sweeper_tx,
            announce_sweeper_tx,
            dreaming_tx,
            config_watcher,
            sweeper_handle,
            announce_sweeper_handle,
            dreaming_handle,
            spawn_controller,
            system_prompt_builder,
            restart_rx,
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
                confirm_flow: &confirm_flow,
                gateway: &gateway,
                slash_registry: &slash_registry,
                shared_cache: &shared_cache,
            },
            &data_dir,
            Arc::clone(&session_config_provider),
        )
        .await?;

        // LLM caller injection: the fallback client was built in layer 2
        // (init_llm_registry → build_fallback_client). Layer 4 wires it
        // into SessionManager (design doc § layer 4).
        let fallback_llm_caller = Arc::new(closeclaw_gateway::llm_caller_impl::FallbackLlmCaller(
            Arc::clone(&fallback_client),
        ));
        // Create SessionMessageHandler for busy/pending state machine.
        let (output_tx, output_rx) = tokio::sync::mpsc::channel(64);
        let active_searcher_llm_caller = Arc::new(
            closeclaw_gateway::session_handler::ActiveSearcherLlmCaller {
                caller: fallback_llm_caller.clone() as Arc<dyn closeclaw_common::LlmCaller>,
                model: String::new(),
            },
        );
        session_manager
            .set_llm_caller(fallback_llm_caller as Arc<dyn closeclaw_common::LlmCaller>)
            .await;
        info!(
            chain_len = fallback_client.chain().len(),
            "LLM call chain injected into SessionManager (layer 4)"
        );
        let session_handler = Arc::new(closeclaw_gateway::SessionMessageHandler::new(
            Arc::clone(&session_manager),
            Arc::clone(&fallback_client),
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
        let (admin_restart_tx, admin_restart_rx) = tokio::sync::mpsc::channel(2);
        let (admin_handle, admin_sock_path) = Self::init_phase_6_admin_rpc(
            &agent_registry,
            &skill_registry,
            &config_manager,
            config_dir,
            admin_restart_tx,
        )
        .await;
        let (chat_handle, chat_sock_path) = Self::init_phase_6_chat_rpc(&gateway, config_dir).await;
        info!(
            "Gateway initialized — CloseClaw daemon started successfully (v{})",
            env!("CARGO_PKG_VERSION")
        );
        Ok(Self {
            gateway: Arc::new(tokio::sync::Mutex::new(gateway)),
            agent_registry,
            permission_engine,
            shutdown,
            session_manager,
            storage,
            sweeper_shutdown_tx: sweeper_tx,
            announce_shutdown_tx: announce_sweeper_tx,
            dreaming_scheduler_shutdown_tx: dreaming_tx,
            skill_registry,
            builtin_skill_registry,
            slash_registry,
            _config_watcher: Some(config_watcher),
            config_watcher_subscriber_handle: None,
            config_manager: Arc::clone(&config_manager),
            approval_flow,
            admin_handle: Some(admin_handle),
            admin_socket_path: admin_sock_path,
            chat_handle: Arc::new(tokio::sync::Mutex::new(Some(chat_handle))),
            chat_socket_path: chat_sock_path,
            archive_sweeper_handle: Some(sweeper_handle),
            announce_sweeper_handle: Some(announce_sweeper_handle),
            dreaming_scheduler_handle: Some(dreaming_handle),
            plan_archive_shutdown_tx,
            plan_archive_sweeper_handle: Some(plan_archive_sweeper_handle),
            media_cleanup_handle,
            spawn_controller: Some(spawn_controller),
            system_prompt_builder: Some(system_prompt_builder),
            llm_registry: Arc::clone(&llm_registry),
            fallback_client: Arc::clone(&fallback_client),
            _output_rx: output_rx,
            restart_state: crate::gateway_restart::RestartHandle::new(),
            restart_rx: Some(restart_rx),
            admin_restart_rx: Some(admin_restart_rx),
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

        // Process restart signals until shutdown is initiated.
        // The restart_rx receives change summaries from DaemonReloadCallback
        // and triggers the restart state machine.
        let mut restart_rx = self.restart_rx.take();
        let mut admin_restart_rx = self.admin_restart_rx.take();
        let mut ready_rx = self.take_restart_ready_rx();
        let mut restart_rx_closed = false;
        let mut admin_restart_rx_closed = false;
        loop {
            tokio::select! {
                biased;
                _ = sigint.recv() => {
                    info!("Received Ctrl+C, initiating graceful shutdown...");
                    self.shutdown.try_start_shutdown();
                    break;
                }
                _ = sigterm.recv() => {
                    info!("Received SIGTERM, initiating graceful shutdown...");
                    self.shutdown.try_start_shutdown();
                    break;
                }
                msg = async { restart_rx.as_mut().unwrap().recv().await }, if !restart_rx_closed => {
                    if let Some(summary) = msg {
                        info!(summary = %summary, "restart signal received from config watcher");
                        let should_spawn = self.request_gateway_restart(vec![summary]);
                        if should_spawn {
                            self.spawn_restart_watchdog();
                        }
                    } else {
                        // Channel closed — receiver taken or sender dropped.
                        restart_rx = None;
                        restart_rx_closed = true;
                    }
                }
                cmd = async { admin_restart_rx.as_mut().unwrap().recv().await }, if !admin_restart_rx_closed => {
                    if let Some(force) = cmd {
                        if force {
                            info!("admin RPC: force restart requested");
                            let (should_spawn, force_pending) =
                                self.force_gateway_restart(vec!["admin force restart".to_string()]);
                            if should_spawn {
                                self.spawn_restart_watchdog();
                            } else if force_pending {
                                self.signal_watchdog_ready(vec!["admin force restart".to_string()]);
                            }
                        } else {
                            info!("admin RPC: cancel pending restart");
                            self.cancel_pending_restart();
                        }
                    } else {
                        admin_restart_rx = None;
                        admin_restart_rx_closed = true;
                    }
                }
                ready = async { ready_rx.as_mut().unwrap().recv().await }, if ready_rx.is_some() => {
                    if let Some(changes) = ready {
                        info!(changes = ?changes, "watchdog ready signal received — executing restart");
                        self.execute_gateway_restart().await;
                    } else {
                        ready_rx = None;
                    }
                }
            }
        }

        // Phase 0: Send brief start notification (no session details yet)
        self.gateway()
            .await
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

    /// Shutdown inbound for all registered IM plugins.
    async fn shutdown_inbound_plugins(gateway: &Arc<closeclaw_gateway::Gateway>) {
        let plugins = gateway.get_all_plugins().await;
        for plugin in &plugins {
            if let Err(e) = plugin.shutdown_inbound().await {
                tracing::warn!(
                    platform = plugin.platform(),
                    error = %e,
                    "failed to shutdown plugin inbound — continuing"
                );
            }
        }
    }

    /// Send a heartbeat card if the interval has elapsed.
    /// Returns `true` if a heartbeat was sent.
    async fn try_send_heartbeat(&self, heartbeat: &mut ShutdownHeartbeat) -> bool {
        if heartbeat.should_send_heartbeat() {
            let mode = self.shutdown.mode();
            tracing::info!(
                elapsed = heartbeat.elapsed_secs(),
                "shutdown heartbeat — sending periodic notification"
            );
            self.gateway()
                .await
                .send_shutdown_heartbeat_card(heartbeat.elapsed_secs(), mode)
                .await;
            heartbeat.record_event();
            return true;
        }
        false
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
        Self::shutdown_inbound_plugins(&self.gateway().await).await;

        let shutdown_handle = self.shutdown.clone();
        let mut shutdown_task =
            tokio::spawn(async move { shutdown_handle.initiate_shutdown().await });
        let mut heartbeat = ShutdownHeartbeat::new();

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
                    heartbeat.record_event();
                }
                _ = sigterm.recv() => {
                    if self.shutdown.escalate_to_forceful() {
                        info!("Received repeated SIGTERM, escalated to forceful shutdown");
                    }
                    heartbeat.record_event();
                }
                _ = tokio::time::sleep_until(heartbeat.next_deadline()) => {
                    self.try_send_heartbeat(&mut heartbeat).await;
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
        self.gateway().await.send_shutdown_progress_card(mode).await;

        // Create progress channel for real-time session stop updates
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<
            closeclaw_gateway::session_manager::stop::StopProgress,
        >(64);

        // Spawn session stop as a background task
        let sm = self.gateway().await.session_manager().clone();
        // Read per-session graceful timeout from config; fall back to DEFAULT_GRACEFUL_TIMEOUT.
        let timeout = {
            use closeclaw_config::providers::SystemConfigData;
            use closeclaw_config::ConfigSection;
            self.config_manager
                .section(ConfigSection::System)
                .and_then(|v| serde_json::from_value::<SystemConfigData>(v).ok())
                .map(|sys| {
                    std::time::Duration::from_secs(sys.effective_shutdown().graceful_timeout_secs)
                })
                .unwrap_or(
                    closeclaw_session::llm_session::session_handles::DEFAULT_GRACEFUL_TIMEOUT,
                )
        };
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
        let mut heartbeat = ShutdownHeartbeat::new();

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
                    heartbeat.record_event();
                    if progress.remaining == 0
                        || now.duration_since(last_card_update) >= throttle_interval
                    {
                        let current_mode: closeclaw_common::shutdown::ShutdownMode =
                            self.shutdown.mode();
                        self.gateway()
                            .await
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

                _ = tokio::time::sleep_until(heartbeat.next_deadline()) => {
                    self.try_send_heartbeat(&mut heartbeat).await;
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
                self.gateway()
                    .await
                    .send_shutdown_progress_card(current_mode)
                    .await;
                last_card_update = std::time::Instant::now();
                last_mode = current_mode;
            }
        }

        // Session stop completed — send final card
        let result = stop_result.unwrap_or_default();
        self.gateway().await.send_shutdown_final_card(&result).await;
        result
    }

    /// Phase 3: Background task stop.
    ///
    /// - Drops ConfigWatcher (RAII) via `take()`, extracting the subscriber
    ///   handle for confirmation
    /// - Signals ArchiveSweeper and DreamingScheduler to stop
    /// - Verifies all 5 background tasks have exited (abort + confirm):
    ///   ArchiveSweeper, AnnounceSweeper, PlanArchiveSweeper,
    ///   DreamingScheduler, ConfigWatcher subscriber
    /// - Clears pending approval requests
    async fn phase_3_background_stop(&mut self) {
        // ConfigWatcher is RAII — stop on drop.
        // Extract the subscriber handle BEFORE dropping so we can
        // join it in wait_all_bg_tasks (design doc: confirm all 5 tasks).
        if let Some(watcher) = self._config_watcher.take() {
            let subscriber = watcher.into_subscriber_handle();
            tracing::info!("ConfigWatcher dropped in Phase 3");
            self.config_watcher_subscriber_handle = Some(subscriber);
        }

        // Signal all background tasks to stop
        let _ = self.sweeper_shutdown_tx.send(());
        let _ = self.announce_shutdown_tx.send(());
        let _ = self.dreaming_scheduler_shutdown_tx.send(());
        let _ = self.plan_archive_shutdown_tx.send(());
        // Stop the media cleanup task (RAII handle, drop signals shutdown).
        if let Some(handle) = self.media_cleanup_handle.take() {
            handle.shutdown();
            tracing::info!("media cleanup task signaled to stop");
        }

        let task_results = self.wait_all_bg_tasks().await;
        Self::log_phase3_stop_confirmation(&task_results);

        // Clear pending approval requests (denied with callbacks triggered)
        self.approval_flow.lock().await.clear();
    }

    /// Wait for all background tasks to exit, sending periodic heartbeats.
    ///
    /// Waits for all 5 background tasks per the design doc:
    /// ArchiveSweeper, AnnounceSweeper, PlanArchiveSweeper,
    /// DreamingScheduler, and ConfigWatcher subscriber.
    async fn wait_all_bg_tasks(&mut self) -> Vec<(&'static str, TaskStopStatus)> {
        let join_timeout = std::time::Duration::from_secs(7);
        let abort_grace = std::time::Duration::from_secs(3);

        // Compile-time: total Phase 3 budget must be 10s per design doc.
        #[allow(clippy::assertions_on_constants, clippy::eq_op)]
        const _: () = assert!(
            7 + 3 == 10,
            "Phase 3 total timeout (join_timeout + abort_grace) must equal 10s"
        );
        let mut heartbeat = ShutdownHeartbeat::new();
        let mut results: Vec<(&str, TaskStopStatus)> = Vec::new();
        let tasks: Vec<(&str, Option<tokio::task::JoinHandle<()>>)> = vec![
            ("ArchiveSweeper", self.archive_sweeper_handle.take()),
            ("AnnounceSweeper", self.announce_sweeper_handle.take()),
            ("DreamingScheduler", self.dreaming_scheduler_handle.take()),
            (
                "PlanArchiveSweeper",
                self.plan_archive_sweeper_handle.take(),
            ),
            (
                "ConfigWatcherSubscriber",
                self.config_watcher_subscriber_handle.take(),
            ),
        ];
        for (name, handle) in tasks {
            if let Some(h) = handle {
                let status = self
                    .wait_for_background_task_with_heartbeat(
                        h,
                        name,
                        join_timeout,
                        abort_grace,
                        &mut heartbeat,
                    )
                    .await;
                results.push((name, status));
            }
        }
        results
    }

    /// Summarize Phase 3 background task stop results.
    fn log_phase3_stop_confirmation(results: &[(&str, TaskStopStatus)]) {
        let clean = results
            .iter()
            .filter(|(_, s)| matches!(s, TaskStopStatus::Clean))
            .count();
        let panicked = results
            .iter()
            .filter(|(_, s)| matches!(s, TaskStopStatus::Panicked))
            .count();
        let aborted = results
            .iter()
            .filter(|(_, s)| matches!(s, TaskStopStatus::Aborted))
            .count();
        info!(
            clean,
            panicked, aborted, "phase 3 background tasks stopped — confirmation"
        );
        for (name, status) in results {
            match status {
                TaskStopStatus::Clean => info!(task = %name, "stopped: clean exit"),
                TaskStopStatus::Panicked => warn!(task = %name, "stopped: panicked"),
                TaskStopStatus::Aborted => {
                    warn!(task = %name, "stopped: aborted (timeout)")
                }
            }
        }
    }

    /// Wait for a background task to exit, sending periodic shutdown
    /// heartbeats during the wait.
    ///
    /// Uses `tokio::select!` with `ShutdownHeartbeat::next_deadline()`
    /// to send heartbeat notifications every 30s while waiting for the
    /// task to finish.  Ref: design doc § "心跳在存在等待的停止阶段
    /// 生效" — Phase 3 后台任务停止.
    async fn wait_for_background_task_with_heartbeat(
        &self,
        mut handle: tokio::task::JoinHandle<()>,
        name: &str,
        timeout: std::time::Duration,
        abort_grace: std::time::Duration,
        heartbeat: &mut ShutdownHeartbeat,
    ) -> TaskStopStatus {
        let wait_with_heartbeats = async {
            loop {
                tokio::select! {
                    result = &mut handle => return result,
                    _ = tokio::time::sleep_until(heartbeat.next_deadline()) => {
                        self.try_send_heartbeat(heartbeat).await;
                    }
                }
            }
        };

        match tokio::time::timeout(timeout, wait_with_heartbeats).await {
            Ok(join_result) => Self::classify_task_result(name, join_result, heartbeat),
            Err(_) => Self::abort_task_with_grace(handle, name, abort_grace, heartbeat).await,
        }
    }

    /// Classify a completed task's join result into a stop status.
    fn classify_task_result(
        name: &str,
        result: Result<(), tokio::task::JoinError>,
        heartbeat: &mut ShutdownHeartbeat,
    ) -> TaskStopStatus {
        match result {
            Ok(()) => {
                info!("{} exited cleanly", name);
                heartbeat.record_event();
                TaskStopStatus::Clean
            }
            Err(e) => {
                warn!(error = %e, "{} task panicked", name);
                heartbeat.record_event();
                TaskStopStatus::Panicked
            }
        }
    }

    /// Abort a task and wait with a grace period for termination.
    async fn abort_task_with_grace(
        handle: tokio::task::JoinHandle<()>,
        name: &str,
        abort_grace: std::time::Duration,
        heartbeat: &mut ShutdownHeartbeat,
    ) -> TaskStopStatus {
        warn!("{} did not exit within timeout, aborting", name);
        handle.abort();
        match tokio::time::timeout(abort_grace, handle).await {
            Ok(Ok(())) => info!("{} terminated after abort", name),
            Ok(Err(_)) => info!("{} task panicked on abort join — terminated", name),
            Err(_) => {
                error!("{} still alive after abort — possible resource leak", name)
            }
        }
        heartbeat.record_event();
        TaskStopStatus::Aborted
    }
    /// Phase 4: Final persistence — two-step fsync to ensure all
    /// session writes are safely persisted.
    ///
    /// 1. Flush session checkpoints — calls [`Gateway::flush_all_sessions`]
    ///    to persist all dirty session state (including any force-mode
    ///    sessions that were not yet flushed during Phase 2).
    /// 2. WAL sync — calls [`Gateway::sync_storage`] to fsync the
    ///    underlying WAL (Write-Ahead Log) so that data reaches stable
    ///    storage.
    ///
    /// These two steps together correspond to the design doc's
    /// "全局 fsync 同步" (global fsync synchronization) for Phase 4.
    async fn phase_4_final_persist(&self, mode: crate::shutdown::ShutdownMode) {
        match self.gateway().await.flush_all_sessions(mode).await {
            Ok(n) => tracing::info!(count = n, mode = ?mode, "flushed session checkpoints"),
            Err(e) => tracing::warn!(error = %e, "failed to flush sessions"),
        }
        match self.gateway().await.sync_storage().await {
            Ok(()) => tracing::info!("storage fsync complete"),
            Err(e) => tracing::warn!(error = %e, "storage fsync failed"),
        }
    }

    /// Phase 5: Outbound shutdown — clean up routing tables.
    async fn phase_5_outbound_close(&self) {
        self.gateway().await.close_outbound().await;
    }

    /// Phase 6: Storage close — release persistent connections/handles.
    async fn phase_6_storage_close(&self) {
        match self.gateway().await.close_storage().await {
            Ok(()) => tracing::info!("storage closed"),
            Err(e) => tracing::warn!(error = %e, "storage close failed"),
        }
    }

    /// Phase 7: Exit cleanup — log warnings, remove admin socket.
    async fn phase_7_exit(&self) {
        // Check for sessions still in the active table — after
        // stop_all_sessions, only sessions that were NOT stopped
        // (e.g. skipped due to missing ConversationSession) remain.
        let remaining = self
            .gateway()
            .await
            .session_manager()
            .get_all_sessions()
            .await;
        let mut stopped_count = 0usize;
        for session in &remaining {
            // Only warn about sessions that haven't been stopped yet.
            let is_stopped = {
                let gw = self.gateway().await;
                let conv = gw.session_manager().conversation_sessions.read().await;
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

/// Stop status of a background task.
#[derive(Debug)]
pub(crate) enum TaskStopStatus {
    /// Task exited cleanly before the join timeout.
    Clean,
    /// Task panicked.
    Panicked,
    /// Task was aborted due to timeout.
    Aborted,
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
    ) -> Arc<closeclaw_slash::registry::HandlerRegistry> {
        use closeclaw_slash::dispatcher::SlashDispatcher;
        use closeclaw_slash::handlers::{ReasoningHandler, SystemHandler, WorkdirHandler};
        use closeclaw_slash::handlers_bg::BackgroundHandler;
        use closeclaw_slash::handlers_permission::PermissionSlashHandler;
        use closeclaw_slash::handlers_user::UserSlashHandler;
        use closeclaw_slash::registry::HandlerRegistry;
        use closeclaw_slash::{
            AutoModeHandler, ClearHandler, CompactHandler, ExecHandler, ExecuteHandler,
            HelpHandler, ModeHandler, NewSessionHandler, PlanBrowseHandler, PlanModeHandler,
            StatusHandler, StopHandler, VerboseHandler,
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
        slash_registry.register(Arc::new(BackgroundHandler::new(Arc::clone(&sm_query))));
        slash_registry.register(Arc::new(PlanBrowseHandler::new(Arc::clone(&sm_query))));
        slash_registry.register(Arc::new(PermissionSlashHandler));
        if let Some(config_dir) = gateway.get_config_dir().await {
            slash_registry.register(Arc::new(UserSlashHandler::new(config_dir)));
        }
        let slash_dispatcher = Arc::new(SlashDispatcher::from_shared(slash_registry))
            as Arc<dyn closeclaw_common::SlashRouter>;
        gateway.set_slash_dispatcher(slash_dispatcher).await;
        // PermissionEngine injection moved to init_phase_3_core_services
        // immediately after Gateway construction (dependency topology aligned).
        info!("Slash dispatcher installed");
        registry_for_return
    }
}
