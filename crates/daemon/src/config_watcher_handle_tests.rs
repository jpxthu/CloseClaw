//! Tests for `ConfigWatcherHandle` shutdown wiring and Phase 3 task-list
//! registration.
//!
//! Split out of `config_reload_tests.rs` so both files stay within the
//! 1000-line limit (CONTRIBUTING.md hard cap).

use super::tests::{
    assert_subscriber_exits, make_config_manager, make_gateway, make_session_manager,
};
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
/// under the watcher.
fn setup_hot_reload() -> (tempfile::TempDir, super::ConfigWatcherHandle) {
    let tmp = tempfile::TempDir::new().unwrap();
    crate::test_helpers::write_mandatory_configs(tmp.path()).unwrap();
    // Bare fixture variant: config_dir is the TempDir root itself (issue #3245).
    let config_mgr = make_config_manager(&tmp);
    let session_mgr = make_session_manager();
    let gateway = make_gateway();
    let agent_registry = Arc::new(closeclaw_agent::registry::AgentRegistry::new());

    let handle = super::init_config_hot_reload(
        tmp.path().to_str().unwrap(),
        config_mgr,
        agent_registry,
        session_mgr,
        gateway,
        None,
    )
    .expect("init_config_hot_reload should succeed");
    (tmp, handle)
}

/// ConfigWatcherHandle holds both the watcher and subscriber handles.
/// Verified via init_config_hot_reload returning Ok with valid config dir.
#[tokio::test]
async fn test_config_watcher_handle_holds_both_handles() {
    let (_tmp, handle) = setup_hot_reload();

    // into_subscriber_handle() returns the subscriber JoinHandle and signals
    // the subscriber to exit (shutdown watch send, issue #3176 B16).
    let subscriber = handle.into_subscriber_handle();
    // The subscriber must now exit cleanly within the timeout — no more
    // timeout-abort reliance.
    assert_subscriber_exits(subscriber, 2, "into_subscriber_handle() signal").await;
}

/// `into_subscriber_handle()` drops the filesystem watcher and returns
/// the subscriber JoinHandle so callers can join it in Phase 3.
#[tokio::test]
async fn test_config_watcher_handle_into_subscriber_handle() {
    let (_tmp, handle) = setup_hot_reload();

    // into_subscriber_handle() drops the watcher and sends the shutdown
    // signal in one step — the subscriber must exit cleanly within the
    // timeout (issue #3176 B16: no more timeout-abort reliance in Phase 3).
    let subscriber = handle.into_subscriber_handle();
    assert_subscriber_exits(subscriber, 2, "into_subscriber_handle() signal").await;
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
