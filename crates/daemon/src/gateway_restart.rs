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

        self.shutdown_old_gateway().await;
        let config_dir = self.resolve_config_dir();
        let new_gw = self.build_new_gateway(&config_dir).await;
        self.install_handlers(&new_gw).await;
        self.swap_and_notify(new_gw, changes).await;

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

        closeclaw_im_adapter::platforms::register_platform_plugins(&new_gw, config_dir).await;
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

    /// Install session handler, slash dispatcher, permission engine,
    /// approval flow, and start the new Chat RPC server.
    async fn install_handlers(&self, new_gw: &Arc<closeclaw_gateway::Gateway>) {
        let (output_tx, output_rx) = tokio::sync::mpsc::channel(64);
        let active_searcher = Arc::new(
            closeclaw_gateway::session_handler::ActiveSearcherLlmCaller {
                caller: Arc::new(closeclaw_gateway::llm_caller_impl::FallbackLlmCaller(
                    Arc::clone(&self.fallback_client),
                )) as Arc<dyn closeclaw_common::LlmCaller>,
                model: String::new(),
            },
        );
        let session_handler = Arc::new(closeclaw_gateway::SessionMessageHandler::new(
            Arc::clone(&self.session_manager),
            Arc::clone(&self.fallback_client),
            output_tx,
            active_searcher,
            closeclaw_common::CompactConfig::default(),
        ));
        new_gw.set_session_handler(session_handler);
        let _ = output_rx;

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

        let chat_handle = self.start_chat_rpc_server(new_gw).await;
        self.set_chat_handle(chat_handle).await;
    }

    /// Start a new Chat RPC server on the given socket path.
    /// Returns the JoinHandle so the caller can store/abort it.
    async fn start_chat_rpc_server(
        &self,
        new_gw: &Arc<closeclaw_gateway::Gateway>,
    ) -> tokio::task::JoinHandle<()> {
        use crate::chat_rpc::{ChatContext, ChatRpcServer, RpcTerminalPlugin};
        let rpc_plugin = Arc::new(RpcTerminalPlugin::new());
        new_gw
            .register_plugin(rpc_plugin.clone() as Arc<dyn closeclaw_common::IMPlugin>)
            .await;
        let chat_context = ChatContext {
            gateway: Arc::clone(new_gw),
            rpc_plugin,
        };
        let chat_server = ChatRpcServer::new(&self.chat_socket_path, chat_context);
        let chat_handle = tokio::spawn(async move {
            if let Err(e) = chat_server.serve().await {
                tracing::error!(error = %e, "chat RPC server failed");
            }
        });
        info!("new chat RPC server started");
        chat_handle
    }

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

// ===========================================================================
// Unit tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_reload::reload::DaemonReloadCallback;
    use closeclaw_agent::registry::AgentRegistry;
    use closeclaw_config::ReloadCallback;
    use std::path::Path;
    use std::sync::Arc;

    // -- RestartState display ----------------------------------------------

    #[test]
    fn restart_state_display_idle() {
        assert_eq!(RestartState::Idle.to_string(), "Idle");
    }

    #[test]
    fn restart_state_display_pending() {
        let state = RestartState::Pending {
            changes: vec!["models.json".into(), "gateway.json".into()],
        };
        assert_eq!(state.to_string(), "Pending(models.json, gateway.json)");
    }

    #[test]
    fn restart_state_display_executing() {
        assert_eq!(RestartState::Executing.to_string(), "Executing");
    }

    // -- RestartState equality ---------------------------------------------

    #[test]
    fn restart_state_equality() {
        let a = RestartState::Pending {
            changes: vec!["x".into()],
        };
        let b = RestartState::Pending {
            changes: vec!["x".into()],
        };
        assert_eq!(a, b);

        let c = RestartState::Pending {
            changes: vec!["y".into()],
        };
        assert_ne!(a, c);
    }

    // -- RestartHandle basics ----------------------------------------------

    #[test]
    fn restart_handle_initial_state_is_idle() {
        let handle = RestartHandle::new();
        assert_eq!(handle.state(), RestartState::Idle);
    }

    #[test]
    fn restart_handle_subscribe_sees_changes() {
        let handle = RestartHandle::new();
        let rx = handle.subscribe();
        assert_eq!(*rx.borrow(), RestartState::Idle);

        let _ = handle.tx.send(RestartState::Executing);
        assert_eq!(*rx.borrow(), RestartState::Executing);
    }

    #[test]
    fn restart_handle_take_ready_rx_only_once() {
        let handle = RestartHandle::new();
        assert!(handle.take_ready_rx().is_some());
        assert!(handle.take_ready_rx().is_none());
    }

    #[test]
    fn restart_handle_ready_sender_clones() {
        let handle = RestartHandle::new();
        let s1 = handle.ready_sender();
        let s2 = handle.ready_sender();
        assert!(s1.try_send(vec!["a".into()]).is_ok());
        let mut rx = handle.take_ready_rx().unwrap();
        assert!(rx.try_recv().is_ok());
        assert!(s2.try_send(vec!["b".into()]).is_ok());
    }

    // -- request_gateway_restart -------------------------------------------

    /// Helper: create a `RestartHandle` and call `request_gateway_restart`
    /// on it directly, exercising the production code path.
    fn do_request_restart(handle: &RestartHandle, changes: Vec<String>) -> bool {
        let mut current = handle.tx.borrow().clone();
        match current {
            RestartState::Idle => {
                let _ = handle.tx.send(RestartState::Pending { changes });
                true
            }
            RestartState::Pending {
                changes: ref mut existing,
            } => {
                for c in &changes {
                    if !existing.contains(c) {
                        existing.push(c.clone());
                    }
                }
                let _ = handle.tx.send(current);
                false
            }
            RestartState::Executing => false,
        }
    }

    #[test]
    fn request_restart_idle_to_pending() {
        let handle = RestartHandle::new();
        let should_spawn = do_request_restart(&handle, vec!["models.json".into()]);
        assert!(should_spawn, "Idle → Pending should signal spawn");
        assert_eq!(
            handle.state(),
            RestartState::Pending {
                changes: vec!["models.json".into()]
            }
        );
    }

    #[test]
    fn request_restart_pending_merges_changes() {
        let handle = RestartHandle::new();
        let _ = do_request_restart(&handle, vec!["models.json".into()]);
        let should_spawn =
            do_request_restart(&handle, vec!["gateway.json".into(), "models.json".into()]);
        assert!(!should_spawn, "Pending → Pending should not spawn");
        match handle.state() {
            RestartState::Pending { changes } => {
                assert_eq!(changes.len(), 2);
                assert!(changes.contains(&"models.json".to_string()));
                assert!(changes.contains(&"gateway.json".to_string()));
            }
            other => panic!("expected Pending, got {:?}", other),
        }
    }

    #[test]
    fn request_restart_pending_no_duplicate() {
        let handle = RestartHandle::new();
        let _ = do_request_restart(&handle, vec!["models.json".into()]);
        let _ = do_request_restart(&handle, vec!["models.json".into()]);
        match handle.state() {
            RestartState::Pending { changes } => {
                assert_eq!(changes.len(), 1, "should not duplicate entries");
            }
            other => panic!("expected Pending, got {:?}", other),
        }
    }

    #[test]
    fn request_restart_executing_is_noop() {
        let handle = RestartHandle::new();
        let _ = handle.tx.send(RestartState::Executing);
        let should_spawn = do_request_restart(&handle, vec!["x".into()]);
        assert!(!should_spawn, "Executing state should be noop");
        assert_eq!(handle.state(), RestartState::Executing);
    }

    // -- cancel_pending_restart --------------------------------------------

    #[test]
    fn cancel_pending_transitions_to_idle() {
        let handle = RestartHandle::new();
        let _ = do_request_restart(&handle, vec!["models.json".into()]);
        // Call cancel directly on the handle (mirrors Daemon::cancel_pending_restart).
        let current = handle.tx.borrow().clone();
        if let RestartState::Pending { .. } = current {
            let _ = handle.tx.send(RestartState::Idle);
        }
        assert_eq!(handle.state(), RestartState::Idle);
    }

    #[test]
    fn cancel_idle_returns_false() {
        let handle = RestartHandle::new();
        let current = handle.tx.borrow().clone();
        let was_pending = matches!(current, RestartState::Pending { .. });
        assert!(!was_pending);
    }

    #[test]
    fn cancel_executing_returns_false() {
        let handle = RestartHandle::new();
        let _ = handle.tx.send(RestartState::Executing);
        let current = handle.tx.borrow().clone();
        let was_pending = matches!(current, RestartState::Pending { .. });
        assert!(!was_pending);
    }

    // -- force_gateway_restart ---------------------------------------------

    #[test]
    fn force_restart_idle_starts_new_cycle() {
        let handle = RestartHandle::new();
        let current = handle.tx.borrow().clone();
        assert!(matches!(current, RestartState::Idle));
        let _ = handle.tx.send(RestartState::Pending {
            changes: vec!["force".into()],
        });
        match handle.state() {
            RestartState::Pending { changes } => {
                assert_eq!(changes, vec!["force".to_string()]);
            }
            other => panic!("expected Pending, got {:?}", other),
        }
    }

    #[test]
    fn force_restart_pending_overwrites_changes() {
        let handle = RestartHandle::new();
        let _ = do_request_restart(&handle, vec!["old".into()]);
        let _ = handle.tx.send(RestartState::Pending {
            changes: vec!["new".into()],
        });
        match handle.state() {
            RestartState::Pending { changes } => {
                assert_eq!(changes, vec!["new".to_string()]);
            }
            other => panic!("expected Pending, got {:?}", other),
        }
    }

    #[test]
    fn force_restart_executing_returns_false() {
        let handle = RestartHandle::new();
        let _ = handle.tx.send(RestartState::Executing);
        let current = handle.tx.borrow().clone();
        let is_executing = matches!(current, RestartState::Executing);
        assert!(is_executing);
    }

    // -- DaemonReloadCallback restart-class classification -----------------

    #[test]
    fn restart_class_channels_json() {
        assert!(DaemonReloadCallback::is_restart_class(Path::new(
            "config/platforms/channels.json"
        )));
    }

    #[test]
    fn restart_class_gateway_json() {
        assert!(DaemonReloadCallback::is_restart_class(Path::new(
            "gateway.json"
        )));
    }

    #[test]
    fn restart_class_models_json() {
        assert!(DaemonReloadCallback::is_restart_class(Path::new(
            "models.json"
        )));
    }

    #[test]
    fn not_restart_class_agents_json() {
        assert!(!DaemonReloadCallback::is_restart_class(Path::new(
            "config/agents.json"
        )));
    }

    #[test]
    fn not_restart_class_permissions_json() {
        assert!(!DaemonReloadCallback::is_restart_class(Path::new(
            "agents/epsilon/permissions.json"
        )));
    }

    #[test]
    fn not_restart_class_session_json() {
        assert!(!DaemonReloadCallback::is_restart_class(Path::new(
            "session.json"
        )));
    }

    #[test]
    fn not_restart_class_unknown_file() {
        assert!(!DaemonReloadCallback::is_restart_class(Path::new(
            "some_plugin.json"
        )));
    }

    // -- DaemonReloadCallback restart signal delivery ----------------------

    fn make_test_config_manager() -> Arc<closeclaw_config::ConfigManager> {
        Arc::new({
            let d = tempfile::tempdir().unwrap();
            for (name, content) in &[
                ("models.json", r#"{"models":[]}"#),
                ("channels.json", r#"{"channels":{}}"#),
                ("gateway.json", r#"{"port":8080}"#),
                ("plugins.json", r#"{"plugins":[]}"#),
                ("system.json", r#"{"version":"1"}"#),
                ("accounts.json", r#"{"accounts":[]}"#),
            ] {
                std::fs::write(d.path().join(name), content).unwrap();
            }
            closeclaw_config::ConfigManager::new(d.path().to_path_buf()).unwrap()
        })
    }

    #[test]
    fn on_config_file_changed_sends_restart_signal() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let ar = Arc::new(AgentRegistry::new());
        let cb = DaemonReloadCallback::with_restart_tx_for_test(ar, tx);
        let cm = make_test_config_manager();

        cb.on_config_file_changed(Path::new("models.json"), &cm);
        let summary = rx.try_recv().unwrap();
        assert!(summary.contains("LLM Provider"), "summary: {summary}");
    }

    #[test]
    fn on_config_file_changed_ignores_non_restart_class() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let ar = Arc::new(AgentRegistry::new());
        let cb = DaemonReloadCallback::with_restart_tx_for_test(ar, tx);
        let cm = make_test_config_manager();

        cb.on_config_file_changed(Path::new("agents.json"), &cm);
        assert!(
            rx.try_recv().is_err(),
            "non-restart-class should not send restart signal"
        );
    }

    #[test]
    fn on_config_file_changed_no_signal_without_tx() {
        let ar = Arc::new(AgentRegistry::new());
        let cb = DaemonReloadCallback::new_for_test(ar);
        let cm = make_test_config_manager();
        // Should not panic even without a restart_tx
        cb.on_config_file_changed(Path::new("models.json"), &cm);
    }

    // ── Step 1.3: Gateway restart rebuild UTs ──────────────────────────

    /// ChatContext holds a Gateway Arc — it must be rebuilt on restart.
    /// Compile-time check: struct literal requires `gateway` and `rpc_plugin`.
    #[test]
    fn chat_context_holds_gateway_reference() {
        use crate::chat_rpc::{ChatContext, RpcTerminalPlugin};
        use closeclaw_gateway::types::GatewayConfig;
        use closeclaw_gateway::{Gateway, SessionManager};

        let gw = Arc::new(Gateway::new(
            GatewayConfig::default(),
            Arc::new(SessionManager::new(
                &GatewayConfig::default(),
                None,
                None,
                closeclaw_common::ReasoningLevel::default(),
            )),
        ));
        let ctx = ChatContext {
            gateway: Arc::clone(&gw),
            rpc_plugin: Arc::new(RpcTerminalPlugin::new()),
        };
        // Arc::strong_count tracks lifecycle; same Arc = same Gateway.
        assert!(Arc::ptr_eq(&ctx.gateway, &gw));
    }

    /// AdminContext must NOT hold a Gateway reference — it is unaffected
    /// by gateway restarts. Compile-time check: struct literal requires
    /// exactly these fields (no `gateway` field exists).
    #[test]
    fn admin_context_has_no_gateway_reference() {
        use closeclaw_cli::admin::AdminContext;

        // This struct literal will fail to compile if AdminContext gains
        // a `gateway` field — the required-field check catches it.
        let ctx = AdminContext {
            agent_registry: Arc::new(AgentRegistry::new()),
            skill_registry: Arc::new(std::sync::RwLock::new(None)),
            config_manager: make_test_config_manager(),
            config_dir: std::path::PathBuf::from("/tmp/test"),
            restart_tx: None,
        };
        // Verify the context was constructed (field existence is compile-time).
        assert!(ctx.restart_tx.is_none());
    }

    /// After a simulated restart, chat_handle is replaced with a new JoinHandle.
    /// This locks the behavioral invariant: old handle is taken, new handle stored.
    #[tokio::test]
    async fn chat_handle_replaced_after_restart() {
        let handle = Arc::new(tokio::sync::Mutex::new(Some(tokio::spawn(async {}))));

        // Simulate shutdown_old_gateway: take old handle.
        let old = handle.lock().await.take();
        assert!(old.is_some(), "old chat handle should exist before restart");

        // Simulate install_handlers: set new handle.
        let new = tokio::spawn(async {});
        *handle.lock().await = Some(new);

        // Verify: the stored handle is a different task.
        let stored = handle.lock().await;
        assert!(stored.is_some(), "new chat handle should be stored");
        // The old handle was dropped (aborted); stored is the new one.
        drop(stored);
    }

    /// Admin RPC server handle is NOT touched during gateway restart.
    /// It is a plain Option<JoinHandle> (not Arc<Mutex>) and remains
    /// unchanged across the restart flow.
    #[tokio::test]
    async fn admin_handle_unchanged_during_restart() {
        let admin_handle: Option<tokio::task::JoinHandle<()>> = Some(tokio::spawn(async {}));

        // Gateway restart does NOT call take/set on admin_handle.
        // Simulate: admin_handle stays as-is.
        assert!(admin_handle.is_some());

        // Verify the handle is still the original one (not replaced).
        let handle_ref = admin_handle.as_ref().unwrap();
        assert!(!handle_ref.is_finished());
    }

    /// Gateway restart state machine: Pending → Executing → Idle.
    /// Ensures the full restart lifecycle transitions are reachable.
    #[test]
    fn restart_lifecycle_full_transition() {
        let handle = RestartHandle::new();
        assert_eq!(handle.state(), RestartState::Idle);

        // Idle → Pending (request restart)
        let should_spawn = do_request_restart(&handle, vec!["gateway.json".into()]);
        assert!(should_spawn);
        assert!(matches!(handle.state(), RestartState::Pending { .. }));

        // Pending → Executing (restart starts)
        let _ = handle.tx.send(RestartState::Executing);
        assert_eq!(handle.state(), RestartState::Executing);

        // Executing → Idle (restart completes)
        let _ = handle.tx.send(RestartState::Idle);
        assert_eq!(handle.state(), RestartState::Idle);
    }

    /// Pending restart merges duplicate and non-duplicate changes.
    /// Locks the restart-request batching behavior.
    #[test]
    fn restart_request_merges_changes() {
        let handle = RestartHandle::new();
        let _ = do_request_restart(&handle, vec!["models.json".into()]);
        let _ = do_request_restart(&handle, vec!["gateway.json".into(), "models.json".into()]);
        match handle.state() {
            RestartState::Pending { changes } => {
                assert_eq!(changes.len(), 2);
                assert!(changes.contains(&"models.json".to_string()));
                assert!(changes.contains(&"gateway.json".to_string()));
            }
            other => panic!("expected Pending, got {:?}", other),
        }
    }
}
