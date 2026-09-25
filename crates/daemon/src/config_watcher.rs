//! Config Hot Reload Initialization
//!
//! Thin initialization entry point for config hot-reload at daemon startup.
//! Delegates file watching and event dispatch to [`ConfigReloadManager`].

use crate::config_reload::DaemonReloadCallback;
use anyhow::Context;
use closeclaw_agent::registry::AgentRegistry;
use closeclaw_config::events::ConfigChangeEvent;
use closeclaw_config::manager::{ConfigManager, ConfigSection, ConfigSnapshot};
use closeclaw_config::providers::SystemConfigData;
use closeclaw_config::{ConfigReloadManager, WatcherHandle};
use closeclaw_gateway::{Gateway, SessionManager};
use std::sync::Arc;
use tokio::sync::watch;
use tracing::{info, warn};

/// RAII handle for the config hot-reload system.
///
/// Dropping this stops the underlying filesystem watcher and signals the
/// subscriber task to shut down (via a `tokio::sync::watch` channel, same
/// pattern as `DreamingScheduler`). The subscriber handle can be used
/// (e.g. in Phase 3) to verify the task has exited.
pub(crate) struct ConfigWatcherHandle {
    _watcher: WatcherHandle,
    shutdown_tx: watch::Sender<bool>,
    _subscriber_handle: tokio::task::JoinHandle<()>,
}

impl ConfigWatcherHandle {
    /// Consume self and return the subscriber JoinHandle.
    ///
    /// The filesystem watcher is dropped here (RAII stop), and the
    /// subscriber is signaled to exit before its handle is returned so
    /// the caller can join it later (e.g. in Phase 3
    /// `wait_all_bg_tasks`). A send failure means the subscriber task
    /// is already gone — safe to ignore, mirroring the sweeper
    /// shutdown-signal pattern.
    pub(crate) fn into_subscriber_handle(self) -> tokio::task::JoinHandle<()> {
        let ConfigWatcherHandle {
            _watcher,
            shutdown_tx,
            _subscriber_handle,
        } = self;
        drop(_watcher);
        // Signal the subscriber to exit (fire-and-forget; a send error
        // means the receiver is already dropped, i.e. the task is gone).
        let _ = shutdown_tx.send(true);
        _subscriber_handle
    }
}

/// Spawn a background task that subscribes to config change events and
/// notifies the [`SessionManager`].
///
/// Receiving the next event and handling it run as one future
/// ([`handle_next_event`]) that is `select!`-raced against the shutdown
/// signal at a single level, so shutdown stays visible while that future
/// is awaiting inside it (no shutdown blind spot).
///
/// Returns the [`JoinHandle`] so the caller can await task completion
/// (e.g. during Phase 3 background-task shutdown).
fn spawn_config_change_subscriber(
    config_manager: Arc<ConfigManager>,
    session_manager: Arc<SessionManager>,
    gateway: Arc<Gateway>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let mut event_rx = config_manager.subscribe_config_changes();
    let mut snapshot_rx = config_manager.subscribe_config_snapshots();
    tokio::spawn(async move {
        loop {
            // Single-level race: receive + handle one event as one future
            // against the shutdown signal — no inner `select!`, one exit
            // decision. Cancellation rationale lives on `handle_next_event`.
            tokio::select! {
                outcome = handle_next_event(
                    &mut event_rx,
                    &config_manager,
                    &session_manager,
                    &gateway,
                    &mut snapshot_rx,
                ) => {
                    if matches!(outcome, EventOutcome::Exit) {
                        break;
                    }
                }
                result = shutdown_rx.changed() => {
                    // Shutdown requested (send(true)) or the sender side was
                    // dropped (RAII drop of ConfigWatcherHandle without
                    // into_subscriber_handle) — either way, exit cleanly.
                    if shutdown_exit_requested(result, &shutdown_rx) {
                        break;
                    }
                }
            }
        }
    })
}

/// Outcome of handling a single config-change event.
#[derive(Debug)]
enum EventOutcome {
    /// Keep waiting for further config-change events.
    Continue,
    /// The config-change broadcast channel closed — exit the subscriber.
    ///
    /// Structural evaluation (issue #3220): defensive, unreachable in
    /// production. The subscriber task holds `Arc<ConfigManager>` for its
    /// whole lifetime and the broadcast sender lives inside that manager,
    /// so the channel cannot close while the loop runs (pinned by
    /// `test_handle_next_event_exits_on_closed_channel` and
    /// `test_subscriber_keeps_config_manager_alive`). The `Arc` holding
    /// is deliberately kept — switching to `Weak` would change lifetime
    /// semantics beyond this behavior-equivalent refactor.
    Exit,
}

/// Receive the next config-change event and handle it: notify sessions of
/// a reload, IM-notify the owner of a failure, absorb lag, and detect
/// broadcast closure.
///
/// Receive and handle run as one future so the subscriber loop can
/// `select!` it against the shutdown signal at a single level: every
/// inner `.await` (the `recv()` itself, snapshot fetch, session
/// notification, owner IM notification) becomes a point where shutdown is
/// observed immediately instead of only once the whole future has
/// completed. Cancelling those awaits is safe by design: cancelling a
/// broadcast `recv()` never loses buffered events (tokio documents
/// `recv()` as cancel-safe), and cancelling a notification drops that
/// notification — matching the shutdown semantics of "stop now, stay on
/// the last valid config".
///
/// Shutdown-side invariant (issue #3220): the shutdown watch channel is
/// only ever driven by `send(true)` or a dropped sender — see
/// [`shutdown_exit_requested`] — so the shutdown branch never cancels
/// this future without the loop exiting afterwards.
async fn handle_next_event(
    event_rx: &mut tokio::sync::broadcast::Receiver<ConfigChangeEvent>,
    config_manager: &ConfigManager,
    session_manager: &SessionManager,
    gateway: &Gateway,
    snapshot_rx: &mut tokio::sync::broadcast::Receiver<ConfigSnapshot>,
) -> EventOutcome {
    match event_rx.recv().await {
        Ok(ConfigChangeEvent::Reloaded { section, .. }) => {
            info!(
                section = %section,
                "config change event received, notifying sessions"
            );
            let snapshot = match snapshot_rx.recv().await {
                Ok(s) => s,
                Err(e) => {
                    warn!(
                        section = %section,
                        error = %e,
                        "failed to receive config snapshot, skipping notification"
                    );
                    return EventOutcome::Continue;
                }
            };
            session_manager
                .notify_config_changed(section, snapshot)
                .await;
        }
        Ok(ConfigChangeEvent::Failed { section, error, .. }) => {
            warn!(
                section = %section,
                error = %error,
                "config change event failed, skipping session notification"
            );
            let target = parse_owner_target(config_manager);
            if let Some((channel, chat_id)) = target {
                let msg = format!(
                    "⚠️ Config reload failed for section `{}`: {}",
                    section, error
                );
                if let Err(e) = gateway
                    .send_outbound_simplified(&chat_id, &channel, &msg)
                    .await
                {
                    warn!(
                        error = %e,
                        "failed to send config reload failure notification to owner"
                    );
                }
            } else {
                warn!(
                    section = %section,
                    "owner_display not configured — skipping IM notification"
                );
            }
        }
        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
            warn!(missed = n, "config change subscriber lagged, missed events");
        }
        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
            // Defensive, production-unreachable — see `EventOutcome::Exit`.
            info!("config change broadcast channel closed, subscriber exiting");
            return EventOutcome::Exit;
        }
    }
    EventOutcome::Continue
}

/// Shared shutdown-exit check for the subscriber loop.
///
/// Returns `true` when the subscriber must exit, emitting a distinct exit
/// log per path so the exit reason is recoverable from logs: an explicit
/// shutdown (`true` sent by `into_subscriber_handle`) logs a received
/// shutdown signal, while a `changed()` error (shutdown sender dropped —
/// RAII drop of `ConfigWatcherHandle` without `into_subscriber_handle`)
/// logs a dropped sender. The explicit-shutdown check runs first so a
/// send-then-drop sequence reports the explicit signal.
///
/// Invariant (issue #3220): production drives this channel with exactly
/// two transitions — `send(true)` from `into_subscriber_handle` (the
/// only production write site) or the sender being dropped (RAII drop
/// of `ConfigWatcherHandle` without `into_subscriber_handle`). Nothing
/// sends `false` on this channel, so a `false` update does not exist in
/// production. Consequently the final `else` (value still `false`, sender still
/// alive) is a defensive arm meaning "no shutdown requested yet", not a
/// supported update: it returns `false` and the loop re-arms its
/// `select!`. Re-arming cancels the in-flight [`handle_next_event`]
/// future (only its `recv()` stage is cancel-safe), so a hypothetical
/// `false` write would abort a partially handled event while leaving the
/// loop alive — that inconsistency is why `false` writes are excluded
/// by invariant; this arm only answers "has shutdown been requested?".
fn shutdown_exit_requested(
    result: Result<(), watch::error::RecvError>,
    shutdown_rx: &watch::Receiver<bool>,
) -> bool {
    if *shutdown_rx.borrow() {
        info!("config change subscriber received shutdown signal, exiting");
        true
    } else if result.is_err() {
        info!("config change subscriber shutdown sender dropped, exiting");
        true
    } else {
        false
    }
}

/// Parse the owner notification target from `SystemConfigData.commands.owner_display`.
fn parse_owner_target(config_manager: &ConfigManager) -> Option<(String, String)> {
    let raw = config_manager
        .get_section_value(ConfigSection::System)
        .and_then(|v| serde_json::from_value::<SystemConfigData>(v).ok())?
        .commands?
        .owner_display?;
    let parts: Vec<&str> = raw.splitn(2, ':').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        warn!(
            owner_display = %raw,
            "invalid owner_display format, expected 'platform:chat_id'"
        );
        return None;
    }
    Some((parts[0].to_string(), parts[1].to_string()))
}

/// Initialize config hot-reload: create a [`ConfigReloadManager`], start the
/// file watcher, and subscribe to config change events.
pub(crate) fn init_config_hot_reload(
    config_dir: &str,
    config_manager: Arc<ConfigManager>,
    agent_registry: Arc<AgentRegistry>,
    session_manager: Arc<SessionManager>,
    gateway: Arc<Gateway>,
    restart_tx: Option<tokio::sync::mpsc::Sender<String>>,
) -> anyhow::Result<ConfigWatcherHandle> {
    let callback = match restart_tx {
        Some(tx) => Arc::new(DaemonReloadCallback::with_restart_tx(
            Arc::clone(&agent_registry),
            tx,
            Arc::clone(&gateway),
        )),
        None => Arc::new(DaemonReloadCallback::new(
            Arc::clone(&agent_registry),
            Arc::clone(&gateway),
        )),
    };
    let mut manager = ConfigReloadManager::with_defaults(Arc::clone(&config_manager), callback);

    let watcher = manager
        .watch(config_dir)
        .context("failed to start config hot-reload watcher")?;

    // Shutdown signal for the subscriber task: watcher drop (Phase 3) →
    // send(true) → subscriber clean exit. Same pattern as
    // DreamingScheduler's tokio::sync::watch shutdown channel.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let subscriber_handle =
        spawn_config_change_subscriber(config_manager, session_manager, gateway, shutdown_rx);

    info!("config hot-reload initialized, delegating to ConfigReloadManager");

    Ok(ConfigWatcherHandle {
        _watcher: watcher,
        shutdown_tx,
        _subscriber_handle: subscriber_handle,
    })
}

#[cfg(test)]
#[path = "config_reload_tests.rs"]
mod tests;

// ConfigWatcherHandle / Phase 3 tests split into a sibling module so both
// files stay within the 1000-line limit (CONTRIBUTING.md hard cap).
#[cfg(test)]
#[path = "config_watcher_handle_tests.rs"]
mod handle_tests;
