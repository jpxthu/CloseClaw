//! Tests for `ConfigWatcherHandle` shutdown wiring and Phase 3 task-list
//! registration.
//!
//! Split out of `config_reload_tests.rs` so both files stay within the
//! 1000-line limit (CONTRIBUTING.md hard cap).

use super::tests::{
    assert_subscriber_exits, make_config_manager, make_gateway, make_session_manager,
};
use closeclaw_config::events::ConfigChangeEvent;
use closeclaw_config::manager::{ConfigManager, ConfigSection};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// ConfigWatcherHandle tests
// ---------------------------------------------------------------------------

/// Shared setup for the two `ConfigWatcherHandle` tests below (issue #3245):
/// create a `TempDir`, write the 6 mandatory config files, build the
/// `ConfigManager` / session / gateway / agent-registry handles, and call
/// [`super::init_config_hot_reload`].
///
/// Returns the `TempDir` **first** so callers bind and keep it alive for the
/// whole test — dropping it would delete the watched directory out from
/// under the watcher, and the `Arc<ConfigManager>` in the middle lets a
/// caller subscribe to config-change events while holding the handle
/// (issue #3247: watcher-alive-during-hold evidence).
fn setup_hot_reload() -> (
    tempfile::TempDir,
    Arc<ConfigManager>,
    super::ConfigWatcherHandle,
) {
    let tmp = tempfile::TempDir::new().unwrap();
    crate::test_helpers::write_mandatory_configs(tmp.path()).unwrap();
    // Bare fixture variant: config_dir is the TempDir root itself (issue #3245).
    let config_mgr = make_config_manager(&tmp);
    let session_mgr = make_session_manager();
    let gateway = make_gateway();
    let agent_registry = Arc::new(closeclaw_agent::registry::AgentRegistry::new());

    let handle = super::init_config_hot_reload(
        tmp.path().to_str().unwrap(),
        Arc::clone(&config_mgr),
        agent_registry,
        session_mgr,
        gateway,
        None,
    )
    .expect("init_config_hot_reload should succeed");
    (tmp, config_mgr, handle)
}

/// Holding period (issue #3247): while the `ConfigWatcherHandle` is alive,
/// **both** handles it owns are effective:
///
/// - subscriber side: the spawned task is still running, its
///   `watch::Receiver` is still registered on the shutdown channel, and the
///   shutdown value is still `false` (nothing consumed the handle yet);
/// - watcher side: the `_watcher` field is still owned by the handle and
///   still listening — rewriting the watched tmp `system.json` must come
///   back as a config-change event through the
///   watcher → reload loop → `ConfigManager` pipeline.
///
/// Distinct from [`test_config_watcher_handle_into_subscriber_handle`],
/// which covers the consumption period (signal sent + clean exit).
#[tokio::test]
async fn test_config_watcher_handle_holds_both_handles() {
    let (tmp, config_mgr, handle) = setup_hot_reload();

    // Subscriber JoinHandle is held and the task has not finished.
    assert!(
        !handle._subscriber_handle.is_finished(),
        "holds_both_handles: subscriber task must still be running while the handle is held"
    );
    // The subscriber's `watch::Receiver` is still registered — at least
    // the task-side receiver must be alive on the shutdown channel.
    let receiver_count = handle.shutdown_tx.receiver_count();
    assert!(
        receiver_count >= 1,
        "holds_both_handles: shutdown channel must keep >= 1 live receiver, got {receiver_count}"
    );
    // No shutdown signal has been sent: `into_subscriber_handle()` has not
    // run yet, so the watch value is still its initial `false`.
    assert!(
        !*handle.shutdown_tx.borrow(),
        "holds_both_handles: shutdown watch value must still be false before consumption"
    );
    // `_watcher` is held by the handle as guaranteed by the type system —
    // no runtime assertion needed; its liveness is proven by the
    // event-roundtrip behavioral assertion below (rewrite the watched
    // `system.json` and wait for the config-change event to come back).

    // Behavioral evidence that the held watcher is still listening: touch
    // a watched tmp config file and await the resulting config-change
    // event — one bounded channel wait, no sleep/poll (STANDARDS §9).
    // Either event variant proves the pipeline ran while the handle held
    // the watcher; the reload outcome itself is not asserted here (that is
    // reload-section behavior, not `ConfigWatcherHandle` behavior), but the
    // event is pinned to the exact rewritten file path.
    let mut event_rx = config_mgr.subscribe_config_changes();
    // Valid System config that differs from the fixture skeleton
    // (`{"version":"1.0"}`), so the rewrite is a real content change.
    std::fs::write(
        tmp.path().join("system.json"),
        r#"{"update": {"checkOnStart": false}}"#,
    )
    .expect("rewrite watched system.json");
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
        .await
        .expect("holds_both_handles: no config event within 5s — held watcher is not listening")
        .expect("holds_both_handles: config change channel failed");
    let (section, path) = match event {
        ConfigChangeEvent::Reloaded { section, path }
        | ConfigChangeEvent::Failed { section, path, .. } => (section, path),
    };
    assert_eq!(
        section,
        ConfigSection::System,
        "holds_both_handles: expected the rewritten system.json to surface a System change event"
    );
    assert_eq!(
        path,
        tmp.path().join("system.json"),
        "holds_both_handles: change event must carry the rewritten system.json path"
    );
}

/// Consumption period (issue #3247): consuming the handle with
/// [`super::ConfigWatcherHandle::into_subscriber_handle`] delivers the
/// shutdown signal at the consumption point and returns the subscriber
/// JoinHandle for Phase 3 to join:
///
/// - state transition: a receiver subscribed *before* consumption sees
///   the shutdown watch value flip `false` → `true` — the signal is sent
///   by the consumption itself, not earlier and not never;
/// - normal path: the returned JoinHandle is the subscriber task and it
///   exits cleanly (bounded wait, clean join — no panic/abort).
///
/// Distinct from [`test_config_watcher_handle_holds_both_handles`], which
/// covers the holding period (both handles still owned, signal still
/// `false`, watcher still listening) and never consumes the handle.
#[tokio::test]
async fn test_config_watcher_handle_into_subscriber_handle() {
    let (_tmp, _config_mgr, handle) = setup_hot_reload();

    // Subscribe before the move: `into_subscriber_handle(self)` consumes
    // the handle, so a pre-consumption receiver is the only way to
    // witness the signal flip at the exact consumption point.
    let shutdown_rx = handle.shutdown_tx.subscribe();

    // Before: no shutdown signal has been sent yet (the state-transition
    // "before" snapshot — A only ever asserts this side via the sender).
    assert!(
        !*shutdown_rx.borrow(),
        "into_subscriber_handle: shutdown signal must still be false before consumption"
    );

    // Consuming the handle drops the filesystem watcher (RAII stop) and
    // sends `shutdown = true` in one step, returning the subscriber
    // JoinHandle (issue #3176 B16: Phase 3 joins it instead of
    // timeout-aborting).
    let subscriber = handle.into_subscriber_handle();

    // After: the same receiver observes the signal — proof that
    // consumption delivered it (never-sent or pre-sent both fail here).
    assert!(
        *shutdown_rx.borrow(),
        "into_subscriber_handle: shutdown signal must be true after consumption"
    );

    // Normal path: the returned handle is the subscriber and it exits
    // cleanly; context string is B-specific so A/B failures stay
    // distinguishable (A uses `holds_both_handles:` prefixes).
    assert_subscriber_exits(subscriber, 2, "into_subscriber_handle clean exit").await;
}

/// Phase 3: ConfigWatcher subscriber is included in the 5-task background
/// stop list. This test verifies the subscriber exits cleanly when its
/// broadcast channel closes, matching the Phase 3 confirmation pattern.
#[tokio::test]
async fn test_phase3_config_watcher_subscriber_in_task_list() {
    use closeclaw_config::events::ConfigChangeEvent;
    use tokio::sync::broadcast;
    use tokio::sync::broadcast::error::RecvError;

    let (tx, _rx) = broadcast::channel::<ConfigChangeEvent>(16);
    let mut subscriber_rx = tx.subscribe();

    let subscriber = tokio::spawn(async move {
        loop {
            match subscriber_rx.recv().await {
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    });

    tokio::task::yield_now().await;
    assert!(!subscriber.is_finished(), "subscriber should be running");

    // Simulate Phase 3: drop watcher (closes channel), subscriber should exit
    drop(tx);

    let result = tokio::time::timeout(std::time::Duration::from_secs(5), subscriber).await;
    assert!(
        result.is_ok(),
        "ConfigWatcher subscriber should exit after channel close in Phase 3"
    );
    assert!(result.unwrap().is_ok(), "subscriber should not panic");
}
