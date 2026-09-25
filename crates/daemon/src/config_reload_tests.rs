//! Tests for daemon config hot-reload module.

use super::*;
use crate::registries::RegistryContext;
use closeclaw_config::events::{ConfigChangeBroadcaster, ConfigChangeEvent};
use closeclaw_config::manager::{ConfigManager, ConfigSection};
use closeclaw_gateway::{Gateway, GatewayConfig, SessionManager};
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::PermissionEngine;
use closeclaw_session::persistence::ReasoningLevel;
use closeclaw_session::tools::LateBoundSessionManagerOps;
use closeclaw_tasks::{
    BackgroundTask, BackgroundTaskError, CompletionNotification, RunningTaskInfo, TaskManager,
};
use closeclaw_tools::ToolRegistry;
use std::sync::{Arc, RwLock};
use tempfile::TempDir;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;

// ── Mock TaskManager ────────────────────────────────────────────────────────

/// Minimal mock implementing [`TaskManager`] for tests that need a task_manager
/// set on [`SessionManager`] (e.g. `populate_registries`).
struct MockTaskManager;

#[async_trait::async_trait]
impl TaskManager for MockTaskManager {
    async fn spawn_task(
        &self,
        _command: &str,
        _cwd: &std::path::Path,
        _is_backgrounded: bool,
        _session_id: &str,
    ) -> Result<BackgroundTask, BackgroundTaskError> {
        unimplemented!("MockTaskManager::spawn_task")
    }
    async fn backgroundize_task(
        &self,
        _child: tokio::process::Child,
        _command: &str,
        _is_backgrounded: bool,
        _session_id: &str,
    ) -> Result<BackgroundTask, BackgroundTaskError> {
        unimplemented!("MockTaskManager::backgroundize_task")
    }
    async fn kill_task(&self, _task_id: &str) -> Result<(), BackgroundTaskError> {
        unimplemented!("MockTaskManager::kill_task")
    }
    async fn get_task(&self, _task_id: &str) -> Option<BackgroundTask> {
        None
    }
    async fn drain_notifications(&self) -> Vec<CompletionNotification> {
        vec![]
    }
    async fn list_running_tasks(&self) -> Vec<RunningTaskInfo> {
        vec![]
    }
    async fn cleanup_all_finished(&self, _session_id: &str) {}
    fn max_execution_secs(&self) -> u64 {
        3600
    }
}

/// Helper: create a ConfigManager backed by a temp directory.
pub(super) fn make_config_manager(tmp: &TempDir) -> Arc<ConfigManager> {
    let config_dir = tmp.path().to_path_buf();
    Arc::new(ConfigManager::new(config_dir).expect("ConfigManager::new should succeed"))
}

/// Helper: create a SessionManager with defaults.
pub(super) fn make_session_manager() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        &GatewayConfig::default(),
        None,
        None,
        ReasoningLevel::default(),
    ))
}

/// Helper: create a Gateway with defaults (for subscriber tests).
pub(super) fn make_gateway() -> Arc<Gateway> {
    Arc::new(Gateway::new(
        GatewayConfig::default(),
        make_session_manager(),
    ))
}

/// Helper: spawn a subscriber test task with a fresh shutdown watch
/// channel (initial state `false`).
///
/// Encapsulates the `spawn_config_change_subscriber()` boilerplate that
/// was repeated across call sites. Returns the shutdown sender and the
/// subscriber [`tokio::task::JoinHandle`]: bind the sender to a named
/// variable (e.g. `_shutdown_tx`) to keep the channel open, and keep the
/// handle to join and assert on subscriber exit (timeout + join, never
/// busy-yield).
pub(super) fn spawn_test_subscriber(
    config_mgr: &Arc<ConfigManager>,
    session_mgr: Arc<SessionManager>,
) -> (watch::Sender<bool>, tokio::task::JoinHandle<()>) {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let subscriber = spawn_config_change_subscriber(
        Arc::clone(config_mgr),
        session_mgr,
        make_gateway(),
        shutdown_rx,
    );
    (shutdown_tx, subscriber)
}

/// Helper: assert that a subscriber task exits cleanly within a bounded
/// time — timeout + join strong assertion, never a busy-yield loop.
///
/// Replaces the three-part inline pattern (`timeout(..).await` → `is_ok()`
/// → join `Ok`) that was repeated at 5 shutdown-exit call sites.
pub(super) async fn assert_subscriber_exits(
    subscriber: tokio::task::JoinHandle<()>,
    timeout_secs: u64,
    context: &str,
) {
    let joined =
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), subscriber).await;
    assert!(
        joined.is_ok(),
        "{context}: must exit within {timeout_secs}s"
    );
    assert!(
        joined.unwrap().is_ok(),
        "{context}: join must be Ok (clean exit, no panic/abort)"
    );
}

// ---------------------------------------------------------------------------
// spawn_config_change_subscriber tests
// ---------------------------------------------------------------------------

/// After a Reloaded event flows into the subscriber, it must exit cleanly
/// within 2s of shutdown being triggered; event completion itself is not
/// asserted.
#[tokio::test]
async fn test_subscriber_handles_reloaded_event() {
    let tmp = TempDir::new().unwrap();
    let config_mgr = make_config_manager(&tmp);
    let session_mgr = make_session_manager();

    let (shutdown_tx, subscriber) = spawn_test_subscriber(&config_mgr, session_mgr);

    // Give the spawned task a moment to start.
    tokio::task::yield_now().await;

    // Send a Reloaded event; delivery/completion is not asserted below.
    config_mgr.notify_change(ConfigChangeEvent::Reloaded {
        section: ConfigSection::Models,
        path: "models.json".into(),
    });

    // Allow the spawned task to process the event.
    tokio::task::yield_now().await;

    // Trigger shutdown and assert a bounded clean exit, so a panic inside
    // the subscriber task surfaces via the join result instead of being
    // silently swallowed by a dropped JoinHandle.
    drop(shutdown_tx);
    assert_subscriber_exits(subscriber, 2, "test_subscriber_handles_reloaded_event").await;
}

/// After a Failed event flows into the subscriber, it must exit cleanly
/// within 2s of shutdown being triggered; logging/notification side
/// effects are not asserted.
#[tokio::test]
async fn test_subscriber_ignores_failed_event() {
    let tmp = TempDir::new().unwrap();
    let config_mgr = make_config_manager(&tmp);
    let session_mgr = make_session_manager();

    let (shutdown_tx, subscriber) = spawn_test_subscriber(&config_mgr, session_mgr);

    tokio::task::yield_now().await;

    // Send a Failed event (logging/notification side effects not asserted).
    config_mgr.notify_change(ConfigChangeEvent::Failed {
        section: ConfigSection::Channels,
        path: "channels.json".into(),
        error: "test parse error".to_string(),
    });

    tokio::task::yield_now().await;

    // Trigger shutdown and assert a bounded clean exit (JoinError observed).
    drop(shutdown_tx);
    assert_subscriber_exits(subscriber, 2, "test_subscriber_ignores_failed_event").await;
}

/// After multiple consecutive events flow into the subscriber, it must
/// exit cleanly within 2s of shutdown being triggered; per-event
/// processing completion is not asserted.
#[tokio::test]
async fn test_subscriber_handles_multiple_events() {
    let tmp = TempDir::new().unwrap();
    let config_mgr = make_config_manager(&tmp);
    let session_mgr = make_session_manager();

    let (shutdown_tx, subscriber) = spawn_test_subscriber(&config_mgr, session_mgr);

    tokio::task::yield_now().await;

    let sections = [
        ConfigSection::Models,
        ConfigSection::Channels,
        ConfigSection::Gateway,
        ConfigSection::Plugins,
        ConfigSection::System,
    ];

    for section in sections {
        config_mgr.notify_change(ConfigChangeEvent::Reloaded {
            section,
            path: section.path(config_mgr.config_dir()),
        });
    }

    // Send a Failed event interleaved.
    config_mgr.notify_change(ConfigChangeEvent::Failed {
        section: ConfigSection::Models,
        path: "models.json".into(),
        error: "interleaved failure".to_string(),
    });

    // More Reloaded events after the failure.
    config_mgr.notify_change(ConfigChangeEvent::Reloaded {
        section: ConfigSection::System,
        path: "system.json".into(),
    });

    // Allow all events to be processed.
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    // Trigger shutdown and assert a bounded clean exit (JoinError observed).
    drop(shutdown_tx);
    assert_subscriber_exits(subscriber, 2, "test_subscriber_handles_multiple_events").await;
}

/// Error path: a closed config-change channel maps to subscriber exit
/// (strong assertion replacing the former 200ms busy-yield with no assert).
///
/// The original end-to-end scenario (spawn subscriber → drop the
/// `ConfigManager` → join under a timeout) is unreachable by construction:
/// the subscriber task owns the last `Arc<ConfigManager>`, and the broadcast
/// sender lives inside it, so the channel cannot close while the subscriber
/// runs — that structural fact is asserted in the companion test
/// `test_subscriber_keeps_config_manager_alive`. The closed-channel exit
/// decision is therefore exercised where it is observable: a genuinely
/// closed config-change channel (its manager dropped) fed to
/// `handle_next_event`, which must report [`EventOutcome::Exit`] — the
/// very value the subscriber loop breaks on.
#[tokio::test]
async fn test_handle_next_event_exits_on_closed_channel() {
    let tmp = TempDir::new().unwrap();
    let config_mgr = make_config_manager(&tmp);
    let session_mgr = make_session_manager();
    let gateway = make_gateway();

    // A real config-change channel, closed by dropping its ConfigManager so
    // `recv()` yields `RecvError::Closed` — exactly what the subscriber's
    // `event_rx` would see if the manager could be dropped underneath it.
    // `recv()` reports `Closed` and keeps reporting it, so the probe assert
    // below and the handler call after it both observe the closed channel.
    let mut closed_event_rx = {
        let doomed_mgr = make_config_manager(&tmp);
        let event_rx = doomed_mgr.subscribe_config_changes();
        drop(doomed_mgr);
        event_rx
    };
    // Pin the exact error variant: `Closed` (all senders dropped) is the
    // subscriber-exit semantic; `Lagged` would mean backlog drop instead.
    let probe = closed_event_rx.recv().await;
    assert!(
        matches!(probe, Err(RecvError::Closed)),
        "expected RecvError::Closed after the manager dropped, got {probe:?}"
    );

    let mut snapshot_rx = config_mgr.subscribe_config_snapshots();
    let outcome = handle_next_event(
        &mut closed_event_rx,
        &config_mgr,
        &session_mgr,
        &gateway,
        &mut snapshot_rx,
    )
    .await;
    assert!(
        matches!(outcome, EventOutcome::Exit),
        "a closed config-change channel must map to EventOutcome::Exit (subscriber break)"
    );
}

/// Structural guard split out of the former
/// `test_subscriber_exits_on_channel_close`: the subscriber task owns the
/// last `Arc<ConfigManager>` and the broadcast sender lives inside it, so
/// the config-change channel can never close while the subscriber runs —
/// which is why the closed-channel exit decision is asserted at the
/// handler boundary instead of on a subscriber-level join.
#[tokio::test]
async fn test_subscriber_keeps_config_manager_alive() {
    let tmp = TempDir::new().unwrap();
    let config_mgr = make_config_manager(&tmp);
    let (_shutdown_tx, subscriber) = spawn_test_subscriber(&config_mgr, make_session_manager());
    tokio::task::yield_now().await;
    drop(config_mgr);
    assert!(
        !subscriber.is_finished(),
        "subscriber owns the last ConfigManager ref: channel close is not observable end-to-end"
    );
}

/// Broadcasting to a channel with no subscriber should not panic.
#[tokio::test]
async fn test_broadcast_no_subscribers_no_panic() {
    let broadcaster = ConfigChangeBroadcaster::new();

    // Sending with no receivers must not panic.
    broadcaster.send(ConfigChangeEvent::Reloaded {
        section: ConfigSection::Models,
        path: "models.json".into(),
    });
    broadcaster.send(ConfigChangeEvent::Failed {
        section: ConfigSection::Channels,
        path: "channels.json".into(),
        error: "test".to_string(),
    });
}

/// Lagged events (subscriber too slow) should be handled gracefully.
#[tokio::test]
async fn test_subscriber_handles_lagged_events() {
    // Use a broadcaster with capacity 1 to easily cause lagging.
    let broadcaster = ConfigChangeBroadcaster::with_capacity(1);
    let mut rx = broadcaster.subscribe();

    // Send many events before the subscriber reads any — some will be lagged.
    for _ in 0..10 {
        broadcaster.send(ConfigChangeEvent::Reloaded {
            section: ConfigSection::Models,
            path: "models.json".into(),
        });
    }

    // Drop the sender so the channel closes after pending events are drained.
    // This prevents recv() from blocking indefinitely once the buffer is empty.
    drop(broadcaster);

    // The subscriber should handle RecvError::Lagged gracefully.
    // Read all pending events to confirm lag actually occurred.
    let mut got_lagged = false;
    loop {
        match rx.recv().await {
            Ok(ConfigChangeEvent::Reloaded { .. }) => {}
            Ok(ConfigChangeEvent::Failed { .. }) => {}
            Err(RecvError::Lagged(_)) => {
                got_lagged = true;
            }
            Err(RecvError::Closed) => break,
        }
    }
    // With capacity 1 and 10 sends, lag must have occurred.
    assert!(
        got_lagged,
        "expected at least one Lagged error with buffer capacity 1 and 10 sends"
    );
}

// ---------------------------------------------------------------------------
// Subscriber shutdown signal behavior tests (issue #3176 B16)
// ---------------------------------------------------------------------------

/// ① Normal path: after `shutdown_tx.send(true)`, the subscriber exits
/// cleanly within a bounded time (join returns `Ok(Ok(()))`) instead of
/// blocking forever on `recv()` and relying on the Phase 3 timeout-abort
/// fallback.
#[tokio::test]
async fn test_subscriber_clean_exit_on_shutdown_signal() {
    let tmp = TempDir::new().unwrap();
    let config_mgr = make_config_manager(&tmp);
    let session_mgr = make_session_manager();

    let (shutdown_tx, subscriber) = spawn_test_subscriber(&config_mgr, session_mgr);

    tokio::task::yield_now().await;
    assert!(
        !subscriber.is_finished(),
        "subscriber should still be running before the shutdown signal"
    );

    shutdown_tx
        .send(true)
        .expect("shutdown sender should still be open");

    assert_subscriber_exits(subscriber, 2, "explicit shutdown signal").await;
}

/// ② Edge: the shutdown signal racing with config events. Events and the
/// shutdown send are issued back-to-back without intervening yields, so the
/// subscriber may observe either `select!` branch first — the event path
/// must not panic and the subscriber must still exit cleanly.
///
/// Note: the Reloaded event is published via `update_section_cache` (the real
/// write path) so the matching snapshot is buffered before the event; a bare
/// `notify_change(Reloaded)` would leave the receive+handle future parked
/// on `snapshot_rx.recv()` for a snapshot that never arrives. Either way
/// that future is raced against shutdown by the subscriber's single
/// `select!`, so the signal is observed without waiting for the event to
/// complete.
#[tokio::test]
async fn test_subscriber_shutdown_signal_concurrent_with_events_no_panic() {
    let tmp = TempDir::new().unwrap();
    let config_mgr = make_config_manager(&tmp);
    let session_mgr = make_session_manager();

    let (shutdown_tx, subscriber) = spawn_test_subscriber(&config_mgr, session_mgr);

    tokio::task::yield_now().await;

    // Concurrent arrival: snapshot+Reloaded event (real write path), a Failed
    // event, then the shutdown signal — all without yields in between.
    config_mgr.update_section_cache(
        ConfigSection::Models,
        tmp.path().join("models.json"),
        serde_json::json!({"version": "2.0"}),
    );
    config_mgr.notify_change(ConfigChangeEvent::Failed {
        section: ConfigSection::Channels,
        path: "channels.json".into(),
        error: "concurrent failure".to_string(),
    });
    let _ = shutdown_tx.send(true);

    assert_subscriber_exits(subscriber, 2, "shutdown signal racing events").await;
}

/// ③ Edge: the shutdown sender is dropped **without** an explicit
/// `send(true)` (RAII drop of `ConfigWatcherHandle` without
/// `into_subscriber_handle()`): the subscriber's single `select!` sees
/// `shutdown_rx.changed()` yield `RecvError::Closed`, and the subscriber
/// must exit cleanly within a bounded time — locking in that a
/// directly-dropped handle leaves no orphan subscriber task behind.
#[tokio::test]
async fn test_subscriber_clean_exit_on_shutdown_sender_drop() {
    let tmp = TempDir::new().unwrap();
    let config_mgr = make_config_manager(&tmp);
    let session_mgr = make_session_manager();

    let (shutdown_tx, subscriber) = spawn_test_subscriber(&config_mgr, session_mgr);

    tokio::task::yield_now().await;
    assert!(
        !subscriber.is_finished(),
        "subscriber should still be running before the sender is dropped"
    );

    // Implicit shutdown: close the watch channel without ever sending `true`.
    drop(shutdown_tx);

    assert_subscriber_exits(subscriber, 2, "shutdown sender dropped").await;
}

/// ④ Regression (E2 Review-B): the shutdown signal must be
/// visible **while** the subscriber is inside the receive+handle future.
///
/// A bare `notify_change(Reloaded)` publishes no snapshot, so
/// `handle_next_event` receives the event and parks on
/// `snapshot_rx.recv()` — the exact inner await that was a shutdown blind
/// spot before the blind-spot fix. `send(true)` issued while parked there
/// must still yield a bounded clean exit (timeout + join): that await
/// lives inside the future raced against shutdown, so the test fails by
/// timeout whenever the await sits outside the race and passes on the
/// current structure.
#[tokio::test]
async fn test_subscriber_shutdown_signal_visible_during_receive_handle() {
    let tmp = TempDir::new().unwrap();
    let config_mgr = make_config_manager(&tmp);
    let session_mgr = make_session_manager();

    let (shutdown_tx, subscriber) = spawn_test_subscriber(&config_mgr, session_mgr);
    tokio::task::yield_now().await;

    // Bare Reloaded event with no snapshot broadcast: the subscriber enters
    // `handle_next_event` and parks waiting for a snapshot that never arrives.
    config_mgr.notify_change(ConfigChangeEvent::Reloaded {
        section: ConfigSection::Models,
        path: "models.json".into(),
    });
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    assert!(
        !subscriber.is_finished(),
        "subscriber should be parked inside the receive+handle future before shutdown"
    );

    shutdown_tx
        .send(true)
        .expect("shutdown sender must still be open");

    assert_subscriber_exits(
        subscriber,
        2,
        "shutdown while parked in receive+handle future",
    )
    .await;
}

// ---------------------------------------------------------------------------
// Gap 2 — IM notification on config reload failure
// ---------------------------------------------------------------------------

/// Shared setup for the `parse_owner_target_*` cases: write `system.json`
/// with the per-case payload, load the `System` section into a fresh
/// [`ConfigManager`], then parse the owner target from it.
///
/// `reload_expect` is the per-case success message for the mandatory
/// reload step, which this helper checks itself; the per-case `assert_eq!`
/// on the parsed value stays in the calling test.
fn parse_owner_target_from(
    system_json: serde_json::Value,
    reload_expect: &str,
) -> Option<(String, String)> {
    let tmp = TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("system.json"),
        serde_json::to_string(&system_json).unwrap(),
    )
    .unwrap();
    // Same construction path as the other tests in this file (config_dir = tmp root).
    let cm = make_config_manager(&tmp);
    // Load only System section (others missing, but we only need System)
    cm.reload_section(ConfigSection::System, None)
        .expect(reload_expect);
    parse_owner_target(&cm)
}

/// parse_owner_target correctly parses a valid owner_display value.
#[test]
fn test_parse_owner_target_valid() {
    let result = parse_owner_target_from(
        serde_json::json!({
            "commands": {
                "ownerDisplay": "feishu:oc_xxx123"
            }
        }),
        "reload system.json with owner_display succeeds",
    );
    assert_eq!(
        result,
        Some(("feishu".to_string(), "oc_xxx123".to_string()))
    );
}

/// parse_owner_target returns None when owner_display is not configured.
#[test]
fn test_parse_owner_target_not_configured() {
    // Payload omits owner_display
    let result = parse_owner_target_from(
        serde_json::json!({ "version": "1.0" }),
        "reload system.json without owner_display succeeds",
    );
    assert_eq!(result, None);
}

/// parse_owner_target returns None for invalid owner_display format.
#[test]
fn test_parse_owner_target_invalid_format() {
    // Missing colon separator
    let result = parse_owner_target_from(
        serde_json::json!({
            "commands": {
                "ownerDisplay": "no-colon-here"
            }
        }),
        "reload system.json with malformed owner_display succeeds",
    );
    assert_eq!(result, None);
}

/// parse_owner_target returns None when owner_display has empty parts.
#[test]
fn test_parse_owner_target_empty_parts() {
    let result = parse_owner_target_from(
        serde_json::json!({
            "commands": {
                "ownerDisplay": ":oc_xxx"
            }
        }),
        "reload system.json with empty owner_display parts succeeds",
    );
    assert_eq!(result, None);
}

/// Failed event with owner_display configured: after the event flows in,
/// the subscriber must exit cleanly within 2s of shutdown being triggered;
/// the in-task UnknownChannel failure path (no IM plugin registered) is
/// not asserted here.
#[tokio::test]
async fn test_subscriber_failed_event_with_owner_display() {
    let tmp = TempDir::new().unwrap();
    // Write system.json with owner_display
    let system_json = serde_json::json!({
        "commands": {
            "ownerDisplay": "feishu:oc_test"
        }
    });
    std::fs::write(
        tmp.path().join("system.json"),
        serde_json::to_string(&system_json).unwrap(),
    )
    .unwrap();
    let config_mgr = make_config_manager(&tmp);
    config_mgr
        .reload_section(ConfigSection::System, None)
        .expect("reload system.json for the owner notification path succeeds");

    let session_mgr = make_session_manager();
    let (shutdown_tx, subscriber) = spawn_test_subscriber(&config_mgr, session_mgr);

    tokio::task::yield_now().await;

    // Send a Failed event with owner_display configured (in-task IM
    // notification path not asserted).
    config_mgr.notify_change(ConfigChangeEvent::Failed {
        section: ConfigSection::Models,
        path: "models.json".into(),
        error: "test failure for IM notification".to_string(),
    });

    // Give the spawned task a chance to run before shutdown.
    tokio::task::yield_now().await;

    // Trigger shutdown and assert a bounded clean exit (JoinError observed).
    drop(shutdown_tx);
    assert_subscriber_exits(
        subscriber,
        2,
        "test_subscriber_failed_event_with_owner_display",
    )
    .await;
}

// ---------------------------------------------------------------------------
// Hot-reload error propagation tests
// ---------------------------------------------------------------------------

/// Shared test harness owning all dependencies required to build a
/// [`RegistryContext`]. Eliminates duplicated setup across tests.
#[cfg(test)]
struct RegistryHarness {
    tmp: TempDir,
    config_mgr: Arc<ConfigManager>,
    skill_registry: Arc<RwLock<Option<closeclaw_skills::DiskSkillRegistry>>>,
    tool_registry: Arc<ToolRegistry>,
    session_mgr: Arc<SessionManager>,
    permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
    gateway: Arc<Gateway>,
    approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
    confirm_flow: Arc<closeclaw_tools::builtin::PlanExecConfirmFlow>,
    builtin_registry: Arc<closeclaw_skills::BuiltinSkillRegistry>,
    agent_registry: Arc<closeclaw_agent::registry::AgentRegistry>,
    spawn_controller: Arc<closeclaw_gateway::SpawnController>,
    late_bound: Arc<LateBoundSessionManagerOps>,
}

/// Build the plan-execution confirm flow sharing `session_mgr` (used by
/// [`RegistryHarness::new`]).
fn make_confirm_flow(
    session_mgr: &Arc<SessionManager>,
) -> Arc<closeclaw_tools::builtin::PlanExecConfirmFlow> {
    Arc::new(
        closeclaw_tools::builtin::PlanExecConfirmFlow::new_without_notify(
            Arc::clone(session_mgr) as Arc<dyn closeclaw_common::SessionLookup>,
            tokio::runtime::Handle::current(),
        ),
    )
}

/// Build the spawn controller wiring config/session/permission deps
/// (used by [`RegistryHarness::new`]).
fn make_spawn_controller(
    config_mgr: &Arc<ConfigManager>,
    session_mgr: &Arc<SessionManager>,
    permission_engine: &Arc<tokio::sync::RwLock<PermissionEngine>>,
) -> Arc<closeclaw_session::spawn::controller::SpawnController> {
    Arc::new({
        let permission_checker: Arc<dyn closeclaw_common::PermissionChecker> = Arc::new(
            closeclaw_gateway::session_manager::spawn_adapter::GatewayPermissionChecker::new(
                Arc::clone(session_mgr),
                Arc::clone(config_mgr),
                Arc::clone(permission_engine),
            ),
        );
        closeclaw_session::spawn::controller::SpawnController::new(
            Arc::clone(config_mgr),
            Arc::clone(session_mgr) as Arc<dyn closeclaw_session::spawn::controller::SpawnContext>,
            permission_checker,
        )
    })
}

#[cfg(test)]
impl RegistryHarness {
    /// Create a harness with default empty skill registry (None).
    /// Sets a mock TaskManager on SessionManager so `populate_registries`
    /// can retrieve it.
    async fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let config_mgr = make_config_manager(&tmp);
        let agent_registry = Arc::new(closeclaw_agent::registry::AgentRegistry::new());
        let skill_registry: Arc<RwLock<Option<closeclaw_skills::DiskSkillRegistry>>> =
            Arc::new(RwLock::new(None));
        let tool_registry = Arc::new(ToolRegistry::new());
        let session_mgr = Arc::new(SessionManager::new(
            &GatewayConfig::default(),
            None,
            None,
            ReasoningLevel::default(),
        ));
        // Set a mock TaskManager so populate_registries can retrieve it.
        session_mgr
            .set_task_manager(Arc::new(MockTaskManager) as Arc<dyn TaskManager>)
            .await;
        let permission_engine = Arc::new(tokio::sync::RwLock::new(
            closeclaw_permission::PermissionEngine::new(
                closeclaw_permission::RuleSet::default(),
                tmp.path().to_path_buf(),
            ),
        ));
        let gateway = make_gateway();
        let approval_flow = Arc::new(tokio::sync::Mutex::new(
            closeclaw_permission::approval_flow::ApprovalFlow::new(
                Arc::clone(&session_mgr) as Arc<dyn closeclaw_common::SessionLookup>,
                Arc::new(|_| {}),
                Arc::new(|_: &str| {}),
                tokio::runtime::Handle::current(),
                closeclaw_permission::approval_flow::HeartbeatApprovalMode::default(),
                tmp.path().to_path_buf(),
                closeclaw_permission::RuleSet::default(),
            ),
        ));
        let builtin_registry = Arc::new(closeclaw_skills::BuiltinSkillRegistry::new());
        let spawn_controller = make_spawn_controller(&config_mgr, &session_mgr, &permission_engine);
        let late_bound = Arc::new(closeclaw_session::tools::LateBoundSessionManagerOps::new());
        let confirm_flow = make_confirm_flow(&session_mgr);

        Self {
            tmp,
            config_mgr,
            skill_registry,
            tool_registry,
            session_mgr,
            permission_engine,
            gateway,
            approval_flow,
            confirm_flow,
            builtin_registry,
            agent_registry,
            spawn_controller,
            late_bound,
        }
    }

    /// Replace the ConfigManager (e.g. after writing mandatory config files
    /// to the temp directory so ConfigManager sees them).
    fn set_config_mgr(&mut self, cm: Arc<ConfigManager>) {
        self.config_mgr = cm;
    }
}

#[cfg(test)]
impl RegistryHarness {
    /// Build a [`RegistryContext`] borrowing from the harness.
    fn ctx(&self) -> RegistryContext<'_> {
        RegistryContext {
            config_manager: &self.config_mgr,
            agent_registry: &self.agent_registry,
            skill_registry: &self.skill_registry,
            builtin_registry: &self.builtin_registry,
            tool_registry: &self.tool_registry,
            session_manager: &self.session_mgr,
            permission_engine: &self.permission_engine,
            spawn_controller: Arc::clone(&self.spawn_controller),
            approval_flow: &self.approval_flow,
            confirm_flow: &self.confirm_flow,
            late_bound_session_manager: Arc::clone(&self.late_bound),
            config_subdir: self.tmp.path(),
            data_dir: self.tmp.path(),
            gateway: &self.gateway,
            restart_tx: None,
        }
    }
}

/// Normal path: `init_config_hot_reload` returns Ok with a valid config dir
/// that contains mandatory config files.
#[tokio::test]
async fn test_hot_reload_init_success_with_valid_config_dir() {
    let tmp = TempDir::new().unwrap();
    // Write mandatory config files so the watcher has something to watch.
    for name in &[
        "models.json",
        "channels.json",
        "gateway.json",
        "plugins.json",
        "system.json",
        "accounts.json",
    ] {
        std::fs::write(
            tmp.path().join(name),
            serde_json::json!({"version": "1.0"}).to_string(),
        )
        .unwrap();
    }
    let config_mgr = make_config_manager(&tmp);
    let session_mgr = make_session_manager();
    let gateway = make_gateway();
    let agent_registry = Arc::new(closeclaw_agent::registry::AgentRegistry::new());

    let result = super::init_config_hot_reload(
        tmp.path().to_str().unwrap(),
        config_mgr,
        agent_registry,
        session_mgr,
        gateway,
        None,
    );
    assert!(
        result.is_ok(),
        "init_config_hot_reload should succeed with valid config dir: {:?}",
        result.err()
    );
}

/// Error path: `populate_registries` returns Err when DiskSkillRegistry is
/// not available, proving the error propagation chain works (Err is returned,
/// not None or silent success).
/// NOTE: This test verifies the DiskSkillRegistry unavailability branch in
/// `populate_registries`. It does NOT test the config hot-reload initialization
/// failure path because `ConfigReloadManager::watch()` is inherently resilient —
/// it only watches files that exist and gracefully skips missing paths, making
/// it impossible to trigger a real watcher failure in a test environment.
#[tokio::test]
async fn test_populate_registries_fails_without_disk_skill_registry() {
    use crate::registries::populate_registries;

    let harness = RegistryHarness::new().await;
    // skill_registry remains None — no DiskSkillRegistry available.
    let ctx = harness.ctx();

    let result = populate_registries(&ctx).await;
    assert!(
        result.is_err(),
        "populate_registries should fail when DiskSkillRegistry is not available"
    );
    let err_msg = match result {
        Err(e) => e.to_string(),
        Ok(_) => unreachable!(),
    };
    assert!(
        err_msg.contains("DiskSkillRegistry"),
        "error message should mention DiskSkillRegistry: {err_msg}"
    );
}

/// Normal path: `populate_registries` returns Ok with a valid config dir
/// and all required registries available.
#[tokio::test]
async fn test_populate_registries_success_with_valid_setup() {
    use crate::registries::populate_registries;

    let mut harness = RegistryHarness::new().await;
    let disk_reg = closeclaw_skills::DiskSkillRegistry::new(vec![]);
    *harness.skill_registry.write().unwrap() = Some(disk_reg);

    // Write mandatory config files so the watcher and ConfigManager load correctly.
    for name in &[
        "models.json",
        "channels.json",
        "gateway.json",
        "plugins.json",
        "system.json",
        "accounts.json",
    ] {
        std::fs::write(
            harness.tmp.path().join(name),
            serde_json::json!({"version": "1.0"}).to_string(),
        )
        .unwrap();
    }
    harness.set_config_mgr(make_config_manager(&harness.tmp));

    let ctx = harness.ctx();
    let result = populate_registries(&ctx).await;
    assert!(
        result.is_ok(),
        "populate_registries should succeed with valid setup: {:?}",
        result.err()
    );
}
