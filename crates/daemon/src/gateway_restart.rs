//! Gateway restart orchestration.
//!
//! Manages the state machine for config-triggered gateway restarts:
//! [`RestartState::Idle`] → [`RestartState::Pending`] → [`RestartState::Executing`].
//!
//! Restart-class config changes are collected in the `Pending` state;
//! the actual rebuild happens in a later step once an idle window is found.

use std::fmt;
use std::sync::Arc;
use tokio::sync::watch;
use tracing::{info, warn};

/// State of the gateway restart lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestartState {
    /// No restart pending — system running normally.
    Idle,
    /// A restart has been requested; `changes` lists affected config paths.
    Pending { changes: Vec<String> },
    /// Gateway rebuild is in progress.
    Executing,
}

impl fmt::Display for RestartState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => write!(f, "Idle"),
            Self::Pending { changes } => {
                write!(f, "Pending({})", changes.join(", "))
            }
            Self::Executing => write!(f, "Executing"),
        }
    }
}

/// Handle to the restart-state watch channel.
///
/// Lightweight handle kept on the [`Daemon`] struct.
/// The receiver half is consumed by the watchdog task (spawned later).
pub(crate) struct RestartHandle {
    tx: watch::Sender<RestartState>,
    /// Keep the initial receiver alive so `tx.send()` always succeeds.
    #[allow(dead_code)]
    _rx: watch::Receiver<RestartState>,
    /// Channel for the watchdog to signal "idle detected, ready to
    /// rebuild".  The sender is held by the watchdog task; the receiver
    /// is consumed by the daemon main loop.
    ready_tx: tokio::sync::mpsc::Sender<Vec<String>>,
    ready_rx: std::sync::Mutex<Option<tokio::sync::mpsc::Receiver<Vec<String>>>>,
}

impl RestartHandle {
    /// Create a handle in the `Idle` state.
    pub(crate) fn new() -> Self {
        let (tx, _rx) = watch::channel(RestartState::Idle);
        let (ready_tx, ready_rx) = tokio::sync::mpsc::channel(1);
        Self {
            tx,
            _rx,
            ready_tx,
            ready_rx: std::sync::Mutex::new(Some(ready_rx)),
        }
    }

    /// Current state snapshot.
    #[allow(dead_code)]
    pub(crate) fn state(&self) -> RestartState {
        self.tx.borrow().clone()
    }

    /// Return a **new** receiver that will see future state changes.
    ///
    /// The caller (watchdog task) should `changed().await` in a loop
    /// to react to transitions.
    pub(crate) fn subscribe(&self) -> watch::Receiver<RestartState> {
        self.tx.subscribe()
    }

    /// Take the ready-receiver (consumed once by the daemon main loop).
    pub(crate) fn take_ready_rx(&self) -> Option<tokio::sync::mpsc::Receiver<Vec<String>>> {
        self.ready_rx.lock().unwrap().take()
    }

    /// Clone the ready-sender for the watchdog task.
    pub(crate) fn ready_sender(&self) -> tokio::sync::mpsc::Sender<Vec<String>> {
        self.ready_tx.clone()
    }
}

// ---------------------------------------------------------------------------
// Daemon methods (impl block)
// ---------------------------------------------------------------------------

impl crate::Daemon {
    /// Request a gateway restart for the given change summaries.
    ///
    /// - If currently **Idle**: transitions to `Pending` and returns
    ///   `true` (caller should spawn the watchdog).
    /// - If currently **Pending**: merges the new `changes` into the
    ///   existing list and returns `false` (watchdog already running).
    /// - If currently **Executing** or **Pending with no new changes**:
    ///   returns `false` — no action needed.
    pub(crate) fn request_gateway_restart(&self, changes: Vec<String>) -> bool {
        let mut current = self.restart_state.tx.borrow().clone();
        match current {
            RestartState::Idle => {
                let new_state = RestartState::Pending { changes };
                let _ = self.restart_state.tx.send(new_state);
                true
            }
            RestartState::Pending {
                changes: ref mut existing,
            } => {
                // Merge: add only non-duplicate entries.
                for c in &changes {
                    if !existing.contains(c) {
                        existing.push(c.clone());
                    }
                }
                let _ = self.restart_state.tx.send(current);
                false
            }
            RestartState::Executing => false,
        }
    }

    /// Cancel a pending restart, returning to `Idle`.
    ///
    /// Returns `true` if a pending restart was cancelled, `false` if
    /// there was nothing to cancel (already Idle or Executing).
    pub(crate) fn cancel_pending_restart(&self) -> bool {
        let current = self.restart_state.tx.borrow().clone();
        match current {
            RestartState::Pending { .. } => {
                let _ = self.restart_state.tx.send(RestartState::Idle);
                true
            }
            _ => false,
        }
    }

    /// Force an immediate gateway restart (skip idle-window wait).
    ///
    /// If currently **Pending**, overwrites changes and signals the
    /// watchdog immediately via the ready channel. If currently **Idle**,
    /// starts a fresh restart cycle by going to `Pending` and returning
    /// `true` (caller should spawn watchdog).
    /// If currently **Executing**, returns `false`.
    ///
    /// Returns `true` if the caller should spawn the watchdog task.
    /// Also sets `force_pending` flag if already Pending, so the
    /// caller can signal the watchdog directly.
    pub(crate) fn force_gateway_restart(&self, changes: Vec<String>) -> (bool, bool) {
        let current = self.restart_state.tx.borrow().clone();
        match current {
            RestartState::Pending { .. } => {
                let _ = self
                    .restart_state
                    .tx
                    .send(RestartState::Pending { changes });
                // Already Pending — caller should signal watchdog
                // directly instead of spawning a new one.
                (false, true)
            }
            RestartState::Idle => {
                let _ = self
                    .restart_state
                    .tx
                    .send(RestartState::Pending { changes });
                (true, false)
            }
            RestartState::Executing => (false, false),
        }
    }

    /// Send the ready signal to the watchdog with the given changes.
    /// Used when `force_gateway_restart` returns `(false, true)` —
    /// the caller needs to signal the already-running watchdog.
    pub(crate) fn signal_watchdog_ready(&self, changes: Vec<String>) {
        let ready_tx = self.restart_state.ready_sender();
        tokio::spawn(async move {
            let _ = ready_tx.send(changes).await;
        });
    }

    /// Resolve the config directory from the admin socket path.
    ///
    /// `admin_socket_path` = `<config_dir>/admin.sock`, so parent()
    /// gives `<config_dir>`. Falls back to current dir on failure.
    fn resolve_config_dir(&self) -> String {
        self.admin_socket_path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string())
    }
}

// ---------------------------------------------------------------------------
// Idle-window detection and Gateway rebuild
// ---------------------------------------------------------------------------

/// Interval (ms) between idle-window checks while a restart is pending.
const WATCHDOG_POLL_INTERVAL_MS: u64 = 10_000;

impl crate::Daemon {
    /// Execute the Gateway rebuild: tear down the old Gateway, create a
    /// new one, re-register all dependencies, and notify the Owner.
    ///
    /// # Invariants
    ///
    /// - [`SessionManager`] is **not** rebuilt — the `Arc<SessionManager>`
    ///   is shared between old and new Gateway, so in-flight session
    ///   state survives the restart.
    /// - The old Gateway's outbound connections (IM plugin websockets,
    ///   webhooks) are shut down before the new Gateway starts.
    /// - The old inbound queue consumer exits naturally when the old
    ///   `Arc<Gateway>` is dropped (its channel sender is dropped).
    pub(crate) async fn execute_gateway_restart(&self) {
        let changes = match self.restart_state.tx.borrow().clone() {
            RestartState::Pending { changes } => changes,
            other => {
                warn!(state = %other, "execute_gateway_restart: unexpected state");
                return;
            }
        };

        info!(changes = ?changes, "starting gateway restart");
        let _ = self.restart_state.tx.send(RestartState::Executing);

        // Enter rebuild mode: subsequent inbound queue-full hits are stashed
        // instead of rejected. Capture the old gateway Arc so we can drain
        // the stash after the swap completes (the Mutex slot will point to
        // the new gateway by then).
        let old_gw = self.gateway().await;
        old_gw.set_rebuild_mode(true);
        info!("old gateway entered rebuild mode");

        self.shutdown_old_gateway().await;
        let config_dir = self.resolve_config_dir();
        let new_gw = self.build_new_gateway(&config_dir).await;
        let _chat_rpc_plugin = self.install_handlers(&new_gw).await;
        self.swap_and_notify(new_gw, changes).await;

        // Replay stashed inbound messages into the new gateway.
        // The old gateway's rebuild_mode is still true, but
        // take_rebuild_stashed only drains the Mutex<VecDeque> — no
        // dependency on the old gateway's live state.
        let stashed = old_gw.take_rebuild_stashed();
        if !stashed.is_empty() {
            let count = stashed.len();
            let new_gw_ref = self.gateway().await;
            let mut replayed = 0u64;
            let mut failed = Vec::new();
            for req in stashed {
                match new_gw_ref.enqueue_inbound(req).await {
                    Ok(()) => replayed += 1,
                    Err(e) => {
                        warn!(
                            trace_id = %e.request.trace_id,
                            peer_id = %e.request.peer_id,
                            "replay: new queue full — returning to stash"
                        );
                        failed.push(e.request);
                    }
                }
            }
            let failed_count = failed.len();
            // Push failed messages back to the old gateway's stash buffer
            // so they are not silently lost.  They remain available for
            // subsequent replay attempts or WAL-based recovery.
            for req in failed {
                old_gw.push_rebuild_stashed(req);
            }
            tracing::info!(
                total = count,
                replayed,
                failed = failed_count,
                "stashed inbound messages replayed"
            );
        }

        // Apply pending restart-class config values after the gateway rebuild
        // completes. This moves staged values from the pending_restart staging
        // area into the runtime cache, making them visible to new sessions and
        // API queries.
        if let Some(config_manager) = self.session_manager.get_config_manager().await {
            config_manager.apply_pending_restart();
            info!("applied pending restart-class config values after gateway restart");
        } else {
            warn!("no config_manager available — skipped apply_pending_restart");
        }
    }

    /// Load GatewayConfig from `{config_dir}/gateway.json`.
    ///
    /// Falls back to `GatewayConfig::default()` if the file is missing
    /// or cannot be parsed.
    async fn load_gateway_config(&self, config_dir: &str) -> closeclaw_gateway::GatewayConfig {
        let config_path = std::path::Path::new(config_dir).join("gateway.json");
        match tokio::fs::read_to_string(&config_path).await {
            Ok(content) => {
                match serde_json::from_str::<closeclaw_gateway::GatewayConfig>(&content) {
                    Ok(config) => {
                        info!("loaded GatewayConfig from {}", config_path.display());
                        config
                    }
                    Err(e) => {
                        warn!(
                            error = %e,
                            path = %config_path.display(),
                            "failed to parse gateway.json — using defaults"
                        );
                        closeclaw_gateway::GatewayConfig::default()
                    }
                }
            }
            Err(e) => {
                warn!(
                    error = %e,
                    path = %config_path.display(),
                    "gateway.json not found — using defaults"
                );
                closeclaw_gateway::GatewayConfig::default()
            }
        }
    }

    /// Shut down the old Gateway: close outbound (IM plugins), stop
    /// old Chat RPC server.
    async fn shutdown_old_gateway(&self) {
        self.gateway().await.close_outbound().await;
        info!("old gateway outbound closed");

        if let Some(handle) = self.take_chat_handle().await {
            handle.abort();
            info!("old chat RPC server stopped");
        }
    }

    /// Create a new Gateway and inject all shared dependencies.
    async fn build_new_gateway(&self, config_dir: &str) -> Arc<closeclaw_gateway::Gateway> {
        let gw_config = self.load_gateway_config(config_dir).await;
        let new_gw = Arc::new(closeclaw_gateway::Gateway::new(
            gw_config,
            Arc::clone(&self.session_manager),
        ));
        new_gw.set_self_ref(Arc::clone(&new_gw));

        new_gw
            .set_config_dir(std::path::PathBuf::from(config_dir))
            .await;
        if let Some(debug_log) = self.gateway().await.get_debug_log() {
            new_gw.set_debug_log(debug_log).await;
        }
        new_gw
            .set_metrics_emitter(Arc::new(closeclaw_common::NoopMetricsEmitter))
            .await;
        let common_sh = crate::bridge::common_shutdown_handle(&self.shutdown);
        new_gw.set_shutdown_handle(Arc::clone(&common_sh));

        // Pass MediaStore and MediaConfigData from the old gateway if available.
        // Re-create MediaStore from config to share the same instance with new gateway.
        let media_config_path = std::path::Path::new(config_dir)
            .join("config")
            .join("media.json");
        let media_config =
            closeclaw_config::MediaConfigData::from_file(&media_config_path).unwrap_or_default();
        let shared_media_store =
            closeclaw_im_adapter::media_store::MediaStore::new(&media_config.storage_dir)
                .ok()
                .map(std::sync::Arc::new);
        closeclaw_im_adapter::platforms::register_platform_plugins(
            &new_gw,
            config_dir,
            shared_media_store,
            Some(media_config),
        )
        .await;
        info!("platform plugins registered on new gateway");

        // Inject shared CheckpointManager from SessionManager so outbound
        // checkpoint persistence survives the restart.
        if let Some(cm) = self.session_manager.checkpoint_manager().await {
            new_gw.set_checkpoint_manager(cm);
            info!("checkpoint manager injected into new gateway");
        } else {
            warn!(
                "session manager has no checkpoint_manager \u{2014} \
                    outbound checkpoint persistence disabled after restart"
            );
        }

        new_gw.start_inbound_queue();
        info!("new inbound queue started");

        new_gw
    }
}

// ---------------------------------------------------------------------------
// Restart-path handler installation (branch-owned turn-completion wiring)
// ---------------------------------------------------------------------------

impl crate::Daemon {
    /// Install session handler, slash dispatcher, permission engine,
    /// approval flow, and start the new Chat RPC server.
    ///
    /// Mirrors the startup path (`lifecycle/mod.rs` / `init_phase_6_chat_rpc`): returns the new
    /// chat RPC server's [`RpcTerminalPlugin`] (the restart call site
    /// drops that handle — the plugin stays alive via gateway
    /// registration, ChatContext and the consumer) and consumes the
    /// SessionMessageHandler output receiver through the shared
    /// [`crate::chat_rpc::spawn_turn_completion_consumer`] — its doc is
    /// the single-point definition of the consumer's semantics.
    async fn install_handlers(
        &self,
        new_gw: &Arc<closeclaw_gateway::Gateway>,
    ) -> Arc<crate::chat_rpc::RpcTerminalPlugin> {
        let (output_tx, output_rx) = tokio::sync::mpsc::channel(64);
        let active_searcher = Arc::new(
            closeclaw_gateway::session_handler::ActiveSearcherLlmCaller {
                caller: Arc::new(closeclaw_gateway::llm_caller_impl::FallbackLlmCaller(
                    Arc::clone(&self.fallback_client),
                )) as Arc<dyn closeclaw_common::LlmCaller>,
                model: String::new(),
            },
        );
        let session_handler = Arc::new(
            closeclaw_gateway::SessionMessageHandler::new(
                Arc::clone(&self.session_manager),
                Arc::clone(&self.fallback_client),
                output_tx,
                active_searcher,
                closeclaw_common::CompactConfig::default(),
            )
            .with_model_knowledge(closeclaw_llm::ProviderModelKnowledge::new()),
        );
        new_gw.set_session_handler(session_handler);

        use closeclaw_slash::dispatcher::SlashDispatcher;
        let slash_dispatcher = Arc::new(SlashDispatcher::from_shared(Arc::clone(
            &self.slash_registry,
        ))) as Arc<dyn closeclaw_common::SlashRouter>;
        new_gw.set_slash_dispatcher(slash_dispatcher).await;
        new_gw
            .set_permission_engine(Arc::clone(&self.permission_engine))
            .await;
        new_gw
            .set_approval_flow(Arc::clone(&self.approval_flow))
            .await;

        let (chat_handle, chat_rpc_plugin) = self.start_chat_rpc_server(new_gw).await;
        self.set_chat_handle(chat_handle).await;

        // Turn-completion consumer for the restart path's own output
        // channel — shared assembly point with the startup path
        // (`init_phase_6_chat_rpc`, Step 1.11): each completed LLM turn
        // finalizes the waiting chat connection instead of letting it
        // time out after 120s.
        crate::chat_rpc::spawn_turn_completion_consumer(output_rx, Arc::clone(&chat_rpc_plugin));
        chat_rpc_plugin
    }

    /// Start a new Chat RPC server on the daemon's chat socket path.
    ///
    /// Assembly is shared with the startup path via
    /// [`crate::chat_rpc::spawn_chat_rpc_server`]. Returns the
    /// join handle (stored so the next restart can abort it) together
    /// with the registered [`RpcTerminalPlugin`] — the caller needs the
    /// plugin handle to drive turn completion.
    async fn start_chat_rpc_server(
        &self,
        new_gw: &Arc<closeclaw_gateway::Gateway>,
    ) -> (
        tokio::task::JoinHandle<()>,
        Arc<crate::chat_rpc::RpcTerminalPlugin>,
    ) {
        crate::chat_rpc::spawn_chat_rpc_server(new_gw, &self.chat_socket_path).await
    }
}

// ---------------------------------------------------------------------------
// Gateway rebuild completion
// ---------------------------------------------------------------------------

impl crate::Daemon {
    /// Swap Gateway references and notify the Owner via IM.
    async fn swap_and_notify(&self, new_gw: Arc<closeclaw_gateway::Gateway>, changes: Vec<String>) {
        self.set_gateway(new_gw).await;

        let _ = self.restart_state.tx.send(RestartState::Idle);
        info!(changes = ?changes, "gateway restart complete");

        let summary = changes.join(", ");
        let gw = self.gateway().await;
        if let Err(e) = gw
            .send_outbound_simplified("owner", "feishu", &summary)
            .await
        {
            warn!(error = %e, "failed to notify owner of gateway restart");
        }
    }

    /// Spawn the watchdog background task that monitors for pending
    /// restarts and triggers execution when an idle window is found.
    ///
    /// The task runs until the daemon shuts down or the restart state
    /// returns to `Idle` / `Executing`.
    pub(crate) fn spawn_restart_watchdog(&self) {
        let mut rx = self.restart_state.subscribe();
        let session_manager = Arc::clone(&self.session_manager);
        let ready_tx = self.restart_state.ready_sender();
        tokio::spawn(async move {
            info!("restart watchdog spawned");
            loop {
                if rx.changed().await.is_err() {
                    info!("restart watchdog: channel closed — exiting");
                    return;
                }
                let state = rx.borrow().clone();
                match state {
                    RestartState::Pending { ref changes } => {
                        let sessions = session_manager.get_all_sessions().await;
                        let all_idle = if sessions.is_empty() {
                            true
                        } else {
                            let mut idle = true;
                            for s in &sessions {
                                let dims = session_manager.activity_dimensions(&s.id).await;
                                if dims.any_active() {
                                    idle = false;
                                    break;
                                }
                            }
                            idle
                        };
                        if all_idle {
                            info!("watchdog: idle window detected — signaling rebuild");
                            let _ = ready_tx.send(changes.clone()).await;
                        } else {
                            tokio::time::sleep(tokio::time::Duration::from_millis(
                                WATCHDOG_POLL_INTERVAL_MS,
                            ))
                            .await;
                        }
                    }
                    RestartState::Idle | RestartState::Executing => {}
                }
            }
        });
    }
}

#[cfg(test)]
#[path = "gateway_restart_tests.rs"]
mod tests;
