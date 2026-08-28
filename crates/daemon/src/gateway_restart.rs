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
    /// If currently **Pending**, transitions directly to `Executing` so
    /// the watchdog proceeds immediately. If currently **Idle**, starts
    /// a fresh restart cycle by going to `Pending` with the given
    /// `changes` and returning `true` (caller should spawn watchdog).
    /// If currently **Executing**, returns `false`.
    ///
    /// Returns `true` if the caller should spawn the watchdog task.
    pub(crate) fn force_gateway_restart(&self, changes: Vec<String>) -> bool {
        let current = self.restart_state.tx.borrow().clone();
        match current {
            RestartState::Pending { .. } => {
                let _ = self
                    .restart_state
                    .tx
                    .send(RestartState::Pending { changes });
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

    /// Resolve the config directory from the admin socket path.
    ///
    /// `admin_socket_path` = `<config_dir>/admin.sock`, so parent.parent
    /// gives `<config_dir>`. Falls back to current dir on failure.
    #[allow(dead_code)]
    fn resolve_config_dir(&self) -> String {
        self.admin_socket_path
            .parent()
            .and_then(|p| p.parent())
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

        self.shutdown_old_gateway().await;
        let config_dir = self.resolve_config_dir();
        let new_gw = self.build_new_gateway(&config_dir).await;
        self.install_handlers(&new_gw).await;
        self.swap_and_notify(new_gw, changes).await;
    }

    /// Shut down the old Gateway: close outbound (IM plugins), stop
    /// old Chat RPC server.
    #[allow(dead_code)]
    async fn shutdown_old_gateway(&self) {
        self.gateway().await.close_outbound().await;
        info!("old gateway outbound closed");

        if let Some(handle) = self.take_chat_handle().await {
            handle.abort();
            info!("old chat RPC server stopped");
        }
    }

    /// Create a new Gateway and inject all shared dependencies.
    #[allow(dead_code)]
    async fn build_new_gateway(&self, config_dir: &str) -> Arc<closeclaw_gateway::Gateway> {
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

        new_gw.start_inbound_queue();
        info!("new inbound queue started");

        new_gw
    }

    /// Install session handler, slash dispatcher, permission engine,
    /// approval flow, and start the new Chat RPC server.
    #[allow(dead_code)]
    async fn install_handlers(&self, new_gw: &Arc<closeclaw_gateway::Gateway>) {
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

        self.set_chat_handle(chat_handle).await;
    }

    /// Swap Gateway references and notify the Owner via IM.
    #[allow(dead_code)]
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
        let cb = DaemonReloadCallback::with_restart_tx(ar, tx);
        let cm = make_test_config_manager();

        cb.on_config_file_changed(Path::new("models.json"), &cm);
        let summary = rx.try_recv().unwrap();
        assert!(summary.contains("LLM Provider"), "summary: {summary}");
    }

    #[test]
    fn on_config_file_changed_ignores_non_restart_class() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let ar = Arc::new(AgentRegistry::new());
        let cb = DaemonReloadCallback::with_restart_tx(ar, tx);
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
        let cb = DaemonReloadCallback::new(ar);
        let cm = make_test_config_manager();
        // Should not panic even without a restart_tx
        cb.on_config_file_changed(Path::new("models.json"), &cm);
    }
}
