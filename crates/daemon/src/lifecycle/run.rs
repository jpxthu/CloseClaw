//! Daemon runtime: startup sequence, run loop, and shutdown phases 0-7.
//!
//! Extracted from `mod.rs` to keep source files within the CONTRIBUTING.md
//! limits (`mod.rs` only holds `pub use` / `pub mod` re-exports).

use crate::shutdown_heartbeat::ShutdownHeartbeat;
use crate::{
    Daemon, Phase2Components, Phase3Components, Phase4Components, Phase5Background, Phase5Deps,
    Phase6RpcServers,
};
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_common::{AuditLogger, CompactConfig, ContentBlock, LlmCaller, SessionLookup};
use closeclaw_config::session::SessionConfigProvider;
use closeclaw_config::ConfigManager;
use closeclaw_gateway::llm_caller_impl::FallbackLlmCaller;
use closeclaw_gateway::session_manager::stop::{StopProgress, StopResult};
use closeclaw_gateway::{Gateway, SessionMessageHandler};
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;
use closeclaw_llm::ProviderModelKnowledge;
use closeclaw_permission::PermissionEngine;
use closeclaw_platform::process;
use closeclaw_session::llm_session::session_handles::DEFAULT_GRACEFUL_TIMEOUT;
use closeclaw_session::storage::SqliteStorage;
use closeclaw_tools::builtin::PlanExecConfirmFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Resolves the effective PID file path for a daemon start: the injected
/// override when present, otherwise the fixed `~/.closeclaw/daemon.pid`
/// resolution via [`process::pid_file_path`]. Pure path computation —
/// no I/O. Mirrors the `SpawnOptions::pid_file_path` injection semantics;
/// the `None` branch must stay exactly the production resolution.
pub(crate) fn resolve_pid_file_path(pid_file_override: Option<&Path>) -> anyhow::Result<PathBuf> {
    match pid_file_override {
        Some(injected) => Ok(injected.to_path_buf()),
        None => process::pid_file_path(),
    }
}

impl Daemon {
    /// Start the daemon with the given config directory.
    pub async fn start(config_dir: &str) -> anyhow::Result<Self> {
        let audit_logger = Self::create_audit_logger(config_dir);
        Self::start_with_engine(config_dir, audit_logger).await
    }
    /// Start the daemon with an injected PID file path (test seam).
    ///
    /// Mirrors `SpawnOptions::pid_file_path` semantics: `None` resolves the
    /// fixed `~/.closeclaw/daemon.pid` via `process::pid_file_path()` —
    /// identical to [`Daemon::start`]; `Some(path)` writes the PID file to
    /// `path` instead so tests can isolate filesystem side effects in a
    /// temporary directory. Production callers use [`Daemon::start`].
    pub async fn start_with_pid_file_path(
        config_dir: &str,
        pid_file_path: Option<PathBuf>,
    ) -> anyhow::Result<Self> {
        let audit_logger = Self::create_audit_logger(config_dir);
        Self::start_with_engine_and_pid_file(config_dir, audit_logger, pid_file_path).await
    }
    /// Start the daemon with an optional audit logger.
    /// If `None`, the engine runs without audit logging.
    pub async fn start_with_engine(
        config_dir: &str,
        audit_logger: Option<Arc<dyn AuditLogger>>,
    ) -> anyhow::Result<Self> {
        Self::start_with_engine_and_pid_file(config_dir, audit_logger, None).await
    }
    async fn start_with_engine_and_pid_file(
        config_dir: &str,
        audit_logger: Option<Arc<dyn AuditLogger>>,
        pid_file_override: Option<PathBuf>,
    ) -> anyhow::Result<Self> {
        info!("Starting CloseClaw daemon with config_dir={}", config_dir);
        Self::load_env(config_dir);
        let pid_file_path = Self::register_pid_file(pid_file_override.as_deref())?;
        let (startup_layers, _phase_components) = Self::resolve_startup_order()?;
        Self::log_startup_order(&startup_layers);
        let (config_manager, storage, data_dir) = Self::init_phase_1_foundation(config_dir)?;
        let p2 = Self::init_phase_2_components(config_dir, &config_manager, &audit_logger).await?;
        let p3 = Self::init_phase_3_components(
            config_dir,
            &storage,
            &p2.permission_engine,
            &config_manager,
        )
        .await?;
        Self::wire_phase3_runtime_references(&p3, &p2.session_config_provider, config_dir).await;
        let p4 =
            Self::init_phase_4_components(&p3, &p2.permission_engine, config_dir, audit_logger)
                .await;
        let confirm_flow = Self::build_plan_exec_confirm_flow(&p3, &config_manager).await;
        let bg =
            Self::init_phase_5_components(&config_manager, &p2, &p3, &p4, &confirm_flow, &data_dir)
                .await?;
        let output_rx = Self::wire_llm_caller_and_session_handler(&p3, &p2.fallback_client).await;
        Self::inject_startup_recovery(&p3).await;
        let rpc =
            Self::init_phase_6_rpc_servers(&p3, &p2, &config_manager, config_dir, output_rx).await;
        info!(
            "Gateway initialized — CloseClaw daemon started successfully (v{})",
            env!("CARGO_PKG_VERSION")
        );
        Ok(Self {
            gateway: Arc::new(tokio::sync::Mutex::new(p3.gateway)),
            agent_registry: p2.agent_registry,
            permission_engine: p2.permission_engine,
            shutdown: p3.shutdown,
            session_manager: p3.session_manager,
            storage,
            sweeper_shutdown_tx: bg.sweeper_shutdown_tx,
            announce_shutdown_tx: bg.announce_shutdown_tx,
            dreaming_scheduler_shutdown_tx: bg.dreaming_scheduler_shutdown_tx,
            skill_registry: p2.skill_registry,
            builtin_skill_registry: p4.builtin_skill_registry,
            slash_registry: p3.slash_registry,
            _config_watcher: Some(bg.config_watcher),
            config_watcher_subscriber_handle: None,
            config_manager: Arc::clone(&config_manager),
            approval_flow: p4.approval_flow,
            confirm_flow,
            admin_handle: Some(rpc.admin_handle),
            admin_socket_path: rpc.admin_socket_path,
            chat_handle: Arc::new(tokio::sync::Mutex::new(Some(rpc.chat_handle))),
            chat_socket_path: rpc.chat_socket_path,
            archive_sweeper_handle: Some(bg.archive_sweeper_handle),
            announce_sweeper_handle: Some(bg.announce_sweeper_handle),
            dreaming_scheduler_handle: Some(bg.dreaming_scheduler_handle),
            plan_archive_shutdown_tx: p2.plan_archive_shutdown_tx,
            plan_archive_sweeper_handle: Some(p2.plan_archive_sweeper_handle),
            media_cleanup_handle: p3.media_cleanup_handle,
            spawn_controller: Some(bg.spawn_controller),
            system_prompt_builder: Some(bg.system_prompt_builder),
            llm_registry: Arc::clone(&p2.llm_registry),
            fallback_client: Arc::clone(&p2.fallback_client),
            restart_state: crate::gateway_restart::RestartHandle::new(),
            restart_rx: Some(bg.restart_rx),
            admin_restart_rx: Some(rpc.admin_restart_rx),
            pid_file_path,
        })
    }
    /// PID self-registration (design doc § PID 自注册).
    ///
    /// Extracted from [`Self::start_with_engine_and_pid_file`]: resolves
    /// the effective PID file path, writes the current PID, and logs (but
    /// does not abort) on write failure. Returns the resolved path so
    /// Phase 7 can clean the file up.
    fn register_pid_file(pid_file_override: Option<&Path>) -> anyhow::Result<PathBuf> {
        let pid_file_path = resolve_pid_file_path(pid_file_override)?;
        if let Err(e) = process::write_pid_file(&pid_file_path, std::process::id()) {
            warn!(
                error = %e,
                pid_file = %pid_file_path.display(),
                "failed to write PID file — daemon will continue without it"
            );
        } else {
            info!(pid_file = %pid_file_path.display(), "PID file written");
        }
        Ok(pid_file_path)
    }
    /// Phase 2: run [`Self::init_phase_2_registries`] and pack the returned
    /// components into [`Phase2Components`].
    ///
    /// Extracted from [`Self::start_with_engine_and_pid_file`].
    async fn init_phase_2_components(
        config_dir: &str,
        config_manager: &ConfigManager,
        audit_logger: &Option<Arc<dyn AuditLogger>>,
    ) -> anyhow::Result<Phase2Components> {
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
        ) = Self::init_phase_2_registries(config_dir, config_manager, audit_logger).await?;
        Ok(Phase2Components {
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
        })
    }
    /// Phase 3: run [`Self::init_phase_3_core_services`] and pack the
    /// returned components into [`Phase3Components`]; the shutdown handle
    /// is Arc-shared here so orchestration steps can clone it while the
    /// original stays on [`Daemon`].
    ///
    /// Extracted from [`Self::start_with_engine_and_pid_file`].
    async fn init_phase_3_components(
        config_dir: &str,
        storage: &Arc<SqliteStorage>,
        permission_engine: &Arc<tokio::sync::RwLock<PermissionEngine>>,
        config_manager: &ConfigManager,
    ) -> anyhow::Result<Phase3Components> {
        let (
            gateway,
            session_manager,
            shutdown,
            dirty_sessions,
            slash_registry,
            media_cleanup_handle,
        ) = Self::init_phase_3_core_services(
            config_dir,
            storage,
            permission_engine,
            config_manager,
        )
        .await?;
        Ok(Phase3Components {
            gateway,
            session_manager,
            shutdown: Arc::new(shutdown),
            dirty_sessions,
            slash_registry,
            media_cleanup_handle,
        })
    }
    /// Wire Phase 3 runtime references: the shutdown handle into Gateway /
    /// SessionManager (busy-count tracking during drain), the independently
    /// created SessionConfigProvider into SessionManager (per-agent
    /// idle/purge thresholds without going through ConfigManager), and
    /// DebugLog into Gateway (config missing or invalid → Gateway runs
    /// without debug logging).
    ///
    /// Extracted from [`Self::start_with_engine_and_pid_file`].
    async fn wire_phase3_runtime_references(
        p3: &Phase3Components,
        session_config_provider: &Arc<dyn SessionConfigProvider>,
        config_dir: &str,
    ) {
        let common_sh = crate::bridge::common_shutdown_handle(&p3.shutdown);
        p3.gateway.set_shutdown_handle(Arc::clone(&common_sh));
        p3.session_manager.set_shutdown_handle(common_sh).await;
        p3.session_manager
            .set_session_config_provider(session_config_provider.clone())
            .await;
        if let Some(debug_log) = Self::init_debug_log(config_dir).await {
            p3.gateway.set_debug_log(debug_log).await;
            info!("DebugLog injected into Gateway");
        }
    }
    /// Phase 4: run [`Self::init_phase_4_wiring`] and pack the wiring
    /// outputs into [`Phase4Components`].
    ///
    /// Extracted from [`Self::start_with_engine_and_pid_file`].
    async fn init_phase_4_components(
        p3: &Phase3Components,
        permission_engine: &Arc<tokio::sync::RwLock<PermissionEngine>>,
        config_dir: &str,
        audit_logger: Option<Arc<dyn AuditLogger>>,
    ) -> Phase4Components {
        let (approval_flow, builtin_skill_registry) = Self::init_phase_4_wiring(
            &p3.gateway,
            &p3.session_manager,
            permission_engine,
            config_dir,
            audit_logger,
        )
        .await;
        Phase4Components {
            approval_flow,
            builtin_skill_registry,
        }
    }
    /// Create PlanExecConfirmFlow for independent plan-execution confirmation.
    ///
    /// Extracted from [`Self::start_with_engine_and_pid_file`]: builds the
    /// flow, wires the child-session factory and the notify callback built
    /// from the gateway's plugins (mirrors set_approval_flow pattern), and
    /// stores it as the gateway's plan confirmation handler.
    async fn build_plan_exec_confirm_flow(
        p3: &Phase3Components,
        config_manager: &Arc<ConfigManager>,
    ) -> Arc<PlanExecConfirmFlow> {
        let sm_lookup: Arc<dyn SessionLookup> =
            Arc::clone(&p3.session_manager) as Arc<dyn SessionLookup>;
        let mut cf =
            PlanExecConfirmFlow::new_without_notify(sm_lookup, tokio::runtime::Handle::current());
        let create_child_fn = Self::build_create_child_fn(
            Arc::clone(&p3.session_manager),
            Arc::clone(config_manager),
        );
        cf.set_create_child_session_fn(create_child_fn);
        let notify_cb = crate::confirm_notify::build_confirm_notify_callback(&p3.gateway).await;
        cf.set_notify_callback(notify_cb);
        let cf = Arc::new(cf);
        // Store as PlanConfirmationHandler on the gateway.
        p3.gateway.set_plan_confirm_handler(cf.clone()).await;
        cf
    }
    /// Phase 5: run [`Self::init_phase_5_background`] — building its
    /// [`Phase5Deps`] bundle from the earlier phase components — and pack
    /// the spawned background services into [`Phase5Background`].
    ///
    /// Extracted from [`Self::start_with_engine_and_pid_file`].
    async fn init_phase_5_components(
        config_manager: &Arc<ConfigManager>,
        p2: &Phase2Components,
        p3: &Phase3Components,
        p4: &Phase4Components,
        confirm_flow: &Arc<PlanExecConfirmFlow>,
        data_dir: &Path,
    ) -> anyhow::Result<Phase5Background> {
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
                config_manager,
                agent_registry: &p2.agent_registry,
                skill_registry: &p2.skill_registry,
                builtin_skill_registry: &p4.builtin_skill_registry,
                tool_registry: &p2.tool_registry,
                session_manager: &p3.session_manager,
                permission_engine: &p2.permission_engine,
                approval_flow: &p4.approval_flow,
                confirm_flow,
                gateway: &p3.gateway,
                slash_registry: &p3.slash_registry,
                shared_cache: &p2.shared_cache,
            },
            data_dir,
            Arc::clone(&p2.session_config_provider),
        )
        .await?;
        Ok(Phase5Background {
            sweeper_shutdown_tx: sweeper_tx,
            announce_shutdown_tx: announce_sweeper_tx,
            dreaming_scheduler_shutdown_tx: dreaming_tx,
            config_watcher,
            archive_sweeper_handle: sweeper_handle,
            announce_sweeper_handle,
            dreaming_scheduler_handle: dreaming_handle,
            spawn_controller,
            system_prompt_builder,
            restart_rx,
        })
    }
    /// LLM caller injection: layer 2's init_llm_registry returns the
    /// registry + UnifiedFallbackClient in one step; layer 4 wires it
    /// into SessionManager (design doc § layer 4).
    ///
    /// Extracted from [`Self::start_with_engine_and_pid_file`]: builds the
    /// SessionMessageHandler (busy/pending state machine), injects the LLM
    /// call chain and handler into SessionManager / Gateway, and returns
    /// the handler's output receiver for the Phase 6 Chat RPC
    /// turn-completion consumer.
    async fn wire_llm_caller_and_session_handler(
        p3: &Phase3Components,
        fallback_client: &Arc<UnifiedFallbackClient>,
    ) -> mpsc::Receiver<(String, Vec<ContentBlock>)> {
        let fallback_llm_caller = Arc::new(FallbackLlmCaller(Arc::clone(fallback_client)));
        // Create SessionMessageHandler for busy/pending state machine.
        let (output_tx, output_rx) = mpsc::channel(64);
        let searcher_runner = crate::searcher_runner::build_searcher_runner(
            fallback_llm_caller.clone() as Arc<dyn LlmCaller>,
        );
        p3.session_manager
            .set_llm_caller(fallback_llm_caller as Arc<dyn LlmCaller>)
            .await;
        info!(
            chain_len = fallback_client.chain().len(),
            "LLM call chain injected into SessionManager (layer 4)"
        );
        let session_handler = Arc::new(
            SessionMessageHandler::new(
                Arc::clone(&p3.session_manager),
                Arc::clone(fallback_client),
                output_tx,
                Some(searcher_runner),
                CompactConfig::default(),
            )
            .with_model_knowledge(ProviderModelKnowledge::new()),
        );
        p3.gateway.set_session_handler(session_handler);
        output_rx
    }
    /// Inject recovery notifications into dirty sessions at startup, then
    /// rebuild the key registry.
    ///
    /// Extracted from [`Self::start_with_engine_and_pid_file`]: must happen
    /// after Phase 5 when LLM caller, system prompt builder, and other
    /// dependencies are wired into SessionManager. Recovery injection may
    /// have created new ConversationSession / Session entries, so the
    /// key_registry is rebuilt to keep them resolvable by routing key.
    async fn inject_startup_recovery(p3: &Phase3Components) {
        p3.session_manager
            .inject_startup_recovery_notifications(&p3.dirty_sessions)
            .await;
        if let Err(e) = p3.session_manager.rebuild_key_registry().await {
            warn!(
                error = %e,
                "failed to rebuild key_registry after recovery injection \
                 — continuing"
            );
        }
    }
    /// Phase 6: start the Admin RPC and Chat RPC servers.
    ///
    /// Extracted from [`Self::start_with_engine_and_pid_file`]: creates the
    /// admin restart command channel, starts both Unix domain socket
    /// servers, and returns their handles/paths plus the admin restart
    /// receiver consumed by the `run()` select loop.
    async fn init_phase_6_rpc_servers(
        p3: &Phase3Components,
        p2: &Phase2Components,
        config_manager: &Arc<ConfigManager>,
        config_dir: &str,
        output_rx: mpsc::Receiver<(String, Vec<ContentBlock>)>,
    ) -> Phase6RpcServers {
        let (admin_restart_tx, admin_restart_rx) = mpsc::channel(2);
        let (admin_handle, admin_sock_path) = Self::init_phase_6_admin_rpc(
            &p2.agent_registry,
            &p2.skill_registry,
            config_manager,
            config_dir,
            admin_restart_tx,
        )
        .await;
        let (chat_handle, chat_sock_path) =
            Self::init_phase_6_chat_rpc(&p3.gateway, config_dir, output_rx).await;
        Phase6RpcServers {
            admin_handle,
            admin_socket_path: admin_sock_path,
            chat_handle,
            chat_socket_path: chat_sock_path,
            admin_restart_rx,
        }
    }
    /// Run the daemon — blocks until shutdown signal is received, then
    /// executes Phase 0–7 shutdown sequence.
    pub async fn run(&mut self) -> anyhow::Result<()> {
        use tokio::signal::unix::SignalKind;

        // Phase 0: Signal reception & mode determination
        // Subscribe to shutdown signals via the platform interface.
        // The subscription wraps reusable streams — after the first
        // signal triggers graceful shutdown, subsequent calls to
        // next_signal() still receive repeated signals during Phase 1
        // drain for escalation.
        let mut shutdown_sub = process::subscribe_shutdown_signals().await?;

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
                result = shutdown_sub.next_signal() => {
                    let kind = match result {
                        Some(k) => k,
                        None => {
                            warn!("shutdown signal streams closed unexpectedly");
                            self.shutdown.try_start_shutdown();
                            break;
                        }
                    };
                    if kind == SignalKind::interrupt() {
                        info!("Received Ctrl+C, initiating graceful shutdown...");
                    } else {
                        info!("Received SIGTERM, initiating graceful shutdown...");
                    }
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

        self.phase_1_inbound_drain(&mut shutdown_sub).await;
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
    async fn shutdown_inbound_plugins(gateway: &Arc<Gateway>) {
        let plugins = gateway.get_all_plugins().await;
        for plugin in &plugins {
            if let Err(e) = plugin.shutdown_inbound().await {
                warn!(
                    platform = plugin.platform(),
                    error = %e,
                    "failed to shutdown plugin inbound — continuing"
                );
            }
        }
    }

    /// Send a heartbeat card if the interval has elapsed.
    /// Returns `true` if a heartbeat was sent.
    pub(crate) async fn try_send_heartbeat(&self, heartbeat: &mut ShutdownHeartbeat) -> bool {
        if heartbeat.should_send_heartbeat() {
            let mode = self.shutdown.mode();
            info!(
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
    async fn phase_1_inbound_drain(&self, shutdown_sub: &mut process::ShutdownSignalSubscription) {
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
                signal = shutdown_sub.next_signal() => {
                    if let Some(kind) = signal {
                        if self.shutdown.escalate_to_forceful() {
                            info!(
                                signal = ?kind,
                                "received repeated signal, escalated to forceful shutdown"
                            );
                        }
                        heartbeat.record_event();
                    }
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
    async fn phase_2_session_stop(&self, mode: ShutdownMode) -> StopResult {
        // Send initial progress card (no-op if no active sessions)
        self.gateway().await.send_shutdown_progress_card(mode).await;

        // Create progress channel for real-time session stop updates
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<StopProgress>(64);

        // Spawn session stop as a background task
        let sm = self.gateway().await.session_manager().clone();
        // Read per-session graceful timeout from config; fall back to DEFAULT_GRACEFUL_TIMEOUT.
        let timeout = {
            use closeclaw_config::providers::SystemConfigData;
            use closeclaw_config::ConfigSection;
            self.config_manager
                .section(ConfigSection::System)
                .and_then(|v| serde_json::from_value::<SystemConfigData>(v).ok())
                .map(|sys| Duration::from_secs(sys.effective_shutdown().graceful_timeout_secs))
                .unwrap_or(DEFAULT_GRACEFUL_TIMEOUT)
        };
        let mut stop_handle = tokio::spawn(async move {
            sm.stop_all_sessions(mode, timeout, Some(&progress_tx))
                .await
        });

        // Spawn a fresh signal subscription for escalation monitoring
        // during Phase 2. Phase 1's subscription is consumed by its
        // tokio::select! loop.
        let mut escalation_sub = process::subscribe_shutdown_signals()
            .await
            .inspect_err(|e| warn!(error = %e, "failed to subscribe for escalation signals"))
            .ok();

        // Heartbeat state: send every 30s when no progress events arrive.
        let mut heartbeat = ShutdownHeartbeat::new();

        // Monitor for escalation and update card
        let mut last_mode = mode;
        let mut stop_completed = false;
        let mut stop_result = None;
        let mut last_card_update = Instant::now();
        let throttle_interval = Duration::from_secs(2);

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
                            stop_result = Some(StopResult::default());
                        }
                    }
                    stop_completed = true;
                }

                Some(progress) = progress_rx.recv() => {
                    // Progress event: update card with throttle
                    let now = Instant::now();
                    heartbeat.record_event();
                    if progress.remaining == 0
                        || now.duration_since(last_card_update) >= throttle_interval
                    {
                        let current_mode: ShutdownMode = self.shutdown.mode();
                        self.gateway()
                            .await
                            .send_shutdown_progress_card(current_mode)
                            .await;
                        last_card_update = now;
                    }
                }

                signal = async {
                    match escalation_sub.as_mut() {
                        Some(sub) => sub.next_signal().await,
                        None => std::future::pending().await,
                    }
                } => {
                    // Escalation signal received
                    if let Some(kind) = signal {
                        if self.shutdown.escalate_to_forceful() {
                            info!(
                                signal = ?kind,
                                "Phase 2: escalated to forceful shutdown"
                            );
                        }
                    }
                }

                _ = tokio::time::sleep_until(heartbeat.next_deadline()) => {
                    self.try_send_heartbeat(&mut heartbeat).await;
                }
            }

            // Check if mode changed and update card
            let current_mode: ShutdownMode = self.shutdown.mode();
            if current_mode != last_mode {
                info!(
                    ?last_mode,
                    ?current_mode,
                    "shutdown mode changed, updating progress card"
                );
                self.gateway()
                    .await
                    .send_shutdown_progress_card(current_mode)
                    .await;
                last_card_update = Instant::now();
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
            info!("ConfigWatcher dropped in Phase 3");
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
            info!("media cleanup task signaled to stop");
        }

        let task_results = self.wait_all_bg_tasks().await;
        Self::log_phase3_stop_confirmation(&task_results);

        // Clear pending approval requests (denied with callbacks triggered)
        self.approval_flow.lock().await.clear();
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
    async fn phase_4_final_persist(&self, mode: ShutdownMode) {
        match self.gateway().await.flush_all_sessions(mode).await {
            Ok(n) => info!(count = n, mode = ?mode, "flushed session checkpoints"),
            Err(e) => warn!(error = %e, "failed to flush sessions"),
        }
        match self.gateway().await.sync_storage().await {
            Ok(()) => info!("storage fsync complete"),
            Err(e) => warn!(error = %e, "storage fsync failed"),
        }
    }

    /// Phase 5: Outbound shutdown — clean up routing tables.
    async fn phase_5_outbound_close(&self) {
        self.gateway().await.close_outbound().await;
    }

    /// Phase 6: Storage close — release persistent connections/handles.
    async fn phase_6_storage_close(&self) {
        match self.gateway().await.close_storage().await {
            Ok(()) => info!("storage closed"),
            Err(e) => warn!(error = %e, "storage close failed"),
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
                warn!(
                    session_id = %session.id,
                    "session still active and not stopped at exit — may need manual recovery"
                );
            }
        }
        if !remaining.is_empty() {
            info!(
                remaining = remaining.len(),
                stopped = stopped_count,
                "phase 7: session table state at exit"
            );
        }
        // Clean up admin socket file
        let _ = tokio::fs::remove_file(&self.admin_socket_path).await;
        // Clean up chat socket file
        let _ = tokio::fs::remove_file(&self.chat_socket_path).await;
        // Clean up PID file (ignore NotFound — already removed or never written)
        match tokio::fs::remove_file(&self.pid_file_path).await {
            Ok(()) => info!(
                pid_file = %self.pid_file_path.display(),
                "PID file removed"
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!(
                    pid_file = %self.pid_file_path.display(),
                    "PID file already absent — no cleanup needed"
                );
            }
            Err(e) => warn!(
                error = %e,
                pid_file = %self.pid_file_path.display(),
                "failed to remove PID file"
            ),
        }
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
