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
    /// Channel for the watchdog to signal "idle detected, ready to
    /// rebuild".  The sender is held by the watchdog task; the receiver
    /// is consumed by the daemon main loop.
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    pub(crate) fn subscribe(&self) -> watch::Receiver<RestartState> {
        self.tx.subscribe()
    }

    /// Take the ready-receiver (consumed once by the daemon main loop).
    pub(crate) fn take_ready_rx(&self) -> Option<tokio::sync::mpsc::Receiver<Vec<String>>> {
        self.ready_rx.lock().unwrap().take()
    }

    /// Clone the ready-sender for the watchdog task.
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
    /// If currently **Pending**, transitions directly to `Executing` so
    /// the watchdog proceeds immediately. If currently **Idle**, starts
    /// a fresh restart cycle by going to `Pending` with the given
    /// `changes` and returning `true` (caller should spawn watchdog).
    /// If currently **Executing**, returns `false`.
    ///
    /// Returns `true` if the caller should spawn the watchdog task.
    #[allow(dead_code)]
    pub(crate) fn force_gateway_restart(&self, changes: Vec<String>) -> bool {
        let current = self.restart_state.tx.borrow().clone();
        match current {
            RestartState::Pending { .. } => {
                // Overwrite with provided changes and signal the watchdog.
                let _ = self
                    .restart_state
                    .tx
                    .send(RestartState::Pending { changes });
                // Tell caller to spawn (or re-spawn) the watchdog.
                true
            }
            RestartState::Idle => {
                let _ = self
                    .restart_state
                    .tx
                    .send(RestartState::Pending { changes });
                true
            }
            RestartState::Executing => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Idle-window detection and Gateway rebuild
// ---------------------------------------------------------------------------

/// Interval (ms) between idle-window checks while a restart is pending.
#[allow(dead_code)]
const WATCHDOG_POLL_INTERVAL_MS: u64 = 10_000;

impl crate::Daemon {
    /// Wait until all sessions are idle (no active LLM calls, no
    /// in-flight inbound processing).
    ///
    /// Polls [`SessionManager::activity_dimensions`] for every known
    /// session every 10 seconds. Returns when all sessions report
    /// `!any_active()`.
    #[allow(dead_code)]
    pub(crate) async fn wait_for_idle_window(&self) {
        loop {
            let sessions = self.session_manager.get_all_sessions().await;
            if sessions.is_empty() {
                info!("idle window: no sessions — proceeding");
                return;
            }
            let all_idle = {
                let mut idle = true;
                for session in &sessions {
                    let dims = self.session_manager.activity_dimensions(&session.id).await;
                    if dims.any_active() {
                        idle = false;
                        break;
                    }
                }
                idle
            };
            if all_idle {
                info!(count = sessions.len(), "idle window: all sessions idle");
                return;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(
                WATCHDOG_POLL_INTERVAL_MS,
            ))
            .await;
        }
    }

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
    #[allow(dead_code)]
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

        // ── 1. Shut down old Gateway outbound (plugins + routing) ──
        self.gateway().await.close_outbound().await;
        info!("old gateway outbound closed");

        // ── 2. Stop old Chat RPC (holds Arc<old Gateway>) ─────────
        // Abort the JoinHandle to cancel the server task.  The socket
        // file is cleaned up by the new server on bind.
        if let Some(handle) = self.take_chat_handle().await {
            handle.abort();
            info!("old chat RPC server stopped");
        }

        // ── 3. Create new Gateway ─────────────────────────────────
        let gw_config = closeclaw_gateway::GatewayConfig {
            name: "closeclaw".to_string(),
            rate_limit_per_minute: 60,
            max_message_size: 16_384,
            ..Default::default()
        };
        let new_gw = Arc::new(closeclaw_gateway::Gateway::new(
            gw_config,
            Arc::clone(&self.session_manager),
        ));
        new_gw.set_self_ref(Arc::clone(&new_gw));

        // ── 4. Re-inject shared dependencies ─────────────────────
        new_gw
            .set_config_dir(std::path::PathBuf::from(
                &self
                    .admin_socket_path
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .parent()
                    .unwrap_or(std::path::Path::new(".")),
            ))
            .await;
        if let Some(debug_log) = self.gateway().await.get_debug_log() {
            new_gw.set_debug_log(debug_log).await;
        }
        new_gw
            .set_metrics_emitter(Arc::new(closeclaw_common::NoopMetricsEmitter))
            .await;
        // ShutdownHandle — shared across Gateway and SessionManager.
        let common_sh = crate::bridge::common_shutdown_handle(&self.shutdown);
        new_gw.set_shutdown_handle(Arc::clone(&common_sh));

        // ── 5. Register platform IM plugins ───────────────────────
        // Extract config_dir from the admin socket path
        // (admin_socket_path = <config_dir>/admin.sock).
        let config_dir = self
            .admin_socket_path
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        closeclaw_im_adapter::platforms::register_platform_plugins(&new_gw, &config_dir).await;
        info!("platform plugins registered on new gateway");

        // ── 6. Start inbound queue ────────────────────────────────
        // The old consumer (holding Arc<old Gateway>) will exit when
        // all Arc references to the old Gateway are dropped.
        new_gw.start_inbound_queue();
        info!("new inbound queue started");

        // ── 7. Install session handler ────────────────────────────
        let (output_tx, output_rx) = tokio::sync::mpsc::channel(64);
        let unified_fallback =
            Arc::new(closeclaw_llm::unified_fallback::UnifiedFallbackClient::new(
                vec![],
                Arc::new(closeclaw_llm::retry::CooldownManager::new()),
            ));
        let active_searcher = Arc::new(
            closeclaw_gateway::session_handler::ActiveSearcherLlmCaller {
                client: Arc::clone(&unified_fallback),
                model: String::new(),
            },
        );
        #[allow(deprecated)]
        let compact_client = Arc::new(closeclaw_llm::fallback::FallbackClient::from_strings(
            Arc::clone(&self.llm_registry),
            vec![],
        ));
        let session_handler = Arc::new(closeclaw_gateway::SessionMessageHandler::new(
            Arc::clone(&self.session_manager),
            compact_client,
            output_tx,
            active_searcher,
            closeclaw_common::CompactConfig::default(),
        ));
        new_gw.set_session_handler(session_handler);
        // Prevent output_rx from being dropped (keeps channel alive).
        let _ = output_rx;

        // ── 8. Install slash dispatcher + permission engine ────────
        use closeclaw_slash::dispatcher::SlashDispatcher;
        let slash_dispatcher = Arc::new(SlashDispatcher::from_shared(Arc::clone(
            &self.slash_registry,
        ))) as Arc<dyn closeclaw_common::SlashRouter>;
        new_gw.set_slash_dispatcher(slash_dispatcher).await;
        new_gw
            .set_permission_engine(Arc::clone(&self.permission_engine))
            .await;

        // ── 9. Install approval flow ──────────────────────────────
        // Must happen after plugins are registered so the approval
        // callback can send messages via the Gateway's plugins.
        new_gw
            .set_approval_flow(Arc::clone(&self.approval_flow))
            .await;

        // ── 10. Start new Chat RPC ───────────────────────────────
        use crate::chat_rpc::{ChatContext, ChatRpcServer, RpcTerminalPlugin};
        let rpc_plugin = Arc::new(RpcTerminalPlugin::new());
        new_gw
            .register_plugin(rpc_plugin.clone() as Arc<dyn closeclaw_common::IMPlugin>)
            .await;
        let chat_context = ChatContext {
            gateway: Arc::clone(&new_gw),
            rpc_plugin,
        };
        let chat_server = ChatRpcServer::new(&self.chat_socket_path, chat_context);
        let chat_handle = tokio::spawn(async move {
            if let Err(e) = chat_server.serve().await {
                tracing::error!(error = %e, "chat RPC server failed");
            }
        });
        info!("new chat RPC server started");

        // ── 11. Swap references ───────────────────────────────────
        // The old Arc<Gateway> may still be alive (held by
        // the old inbound consumer), but it will exit once its
        // channel sender is dropped.
        self.set_gateway(new_gw).await;
        self.set_chat_handle(chat_handle).await;

        // ── 12. Set state to Idle ─────────────────────────────────
        let _ = self.restart_state.tx.send(RestartState::Idle);
        info!(changes = ?changes, "gateway restart complete");

        // ── 13. Notify Owner via IM ──────────────────────────────
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
    #[allow(dead_code)]
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
                            let _ = ready_tx.send(changes.clone()).await;
                        }
                    }
                    RestartState::Idle | RestartState::Executing => {}
                }
            }
        });
    }
}
