//! Behavior tests for the flattened subscriber `select!` structure
//! (issue #3220): `handle_next_event` (recv + handle as one future) is
//! raced against `shutdown_rx.changed()` at a single level.
//!
//! Added as a new sibling test module (nothing was split out of
//! `config_reload_tests.rs`) so that file stays within the 1000-line
//! limit (CONTRIBUTING.md hard cap).
//!
//! Dimensions covered here:
//! - normal path: `Reloaded` / `Failed` events are handled → `Continue`
//!   (a snapshot-buffer oracle tells the two branches apart)
//! - error path: lagged broadcast → `Continue`, then the retained event
//!   is still fully handled
//! - false-update contract: `send(false)` keeps the loop running and the
//!   decision function keeps answering "no shutdown requested yet"
//!
//! Same plan dimensions already covered by `config_reload_tests.rs`
//! (deliberately not duplicated): closed channel → `EventOutcome::Exit`
//! (`test_handle_next_event_exits_on_closed_channel`), shutdown racing an
//! in-flight event (`test_subscriber_shutdown_signal_concurrent_with_events_no_panic`,
//! `test_subscriber_shutdown_signal_visible_inside_event_branch`) and
//! dropped shutdown sender → clean exit
//! (`test_subscriber_clean_exit_on_shutdown_sender_drop`).

use super::tests::{
    assert_subscriber_exits, make_config_manager, make_gateway, make_session_manager,
    spawn_test_subscriber,
};
use super::*;
use closeclaw_config::events::{ConfigChangeBroadcaster, ConfigChangeEvent};
use closeclaw_config::manager::ConfigSection;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::broadcast::error::TryRecvError;

// ---------------------------------------------------------------------------
// Normal path — Reloaded / Failed events → Continue
// ---------------------------------------------------------------------------

/// Normal path: a `Reloaded` event is fully handled by
/// [`handle_next_event`] — the buffered snapshot is consumed (snapshot
/// fetch + session notification ran) and the flattened future reports
/// [`EventOutcome::Continue`].
///
/// The snapshot-consumption assert is what makes this meaningful: the
/// early "snapshot missing" return also yields `Continue`, so without it
/// the test could not distinguish full handling from a skipped one.
#[tokio::test]
async fn test_handle_next_event_reloaded_handles_then_continues() {
    let tmp = TempDir::new().unwrap();
    let config_mgr = make_config_manager(&tmp);
    let session_mgr = make_session_manager();
    let gateway = make_gateway();

    let mut event_rx = config_mgr.subscribe_config_changes();
    let mut snapshot_rx = config_mgr.subscribe_config_snapshots();

    // Real write path: broadcasts the matching snapshot before the event.
    config_mgr.update_section_cache(
        ConfigSection::Models,
        tmp.path().join("models.json"),
        serde_json::json!({ "version": "2.0" }),
    );

    let outcome = tokio::time::timeout(
        Duration::from_secs(2),
        handle_next_event(
            &mut event_rx,
            &config_mgr,
            &session_mgr,
            &gateway,
            &mut snapshot_rx,
        ),
    )
    .await
    .expect("Reloaded handling must complete instead of parking");

    assert!(
        matches!(outcome, EventOutcome::Continue),
        "Reloaded event must be handled and report Continue, got {outcome:?}"
    );
    assert!(
        matches!(snapshot_rx.try_recv(), Err(TryRecvError::Empty)),
        "Reloaded branch must consume the buffered snapshot (full handling, not an early Continue)"
    );
    assert!(
        matches!(event_rx.try_recv(), Err(TryRecvError::Empty)),
        "Reloaded event must be consumed from the event stream"
    );
}

/// Normal path: a `Failed` event takes the owner-notification branch and
/// the future reports [`EventOutcome::Continue`]. `owner_display` is
/// configured, so `parse_owner_target` yields a target and the gateway
/// outbound attempt runs (no IM plugin registered → plain-text
/// fallback): the path is exercised but not directly observed — the
/// bounded await only proves it completes without hanging or panicking.
///
/// Asserted scope: the event is consumed and the buffered snapshot is
/// left untouched (a Reloaded branch would consume one), which separates
/// this test from `test_handle_next_event_reloaded_handles_then_continues`;
/// the notification itself is not asserted.
#[tokio::test]
async fn test_handle_next_event_failed_notifies_owner_then_continues() {
    let tmp = TempDir::new().unwrap();
    // owner_display configured → the owner IM notification path runs.
    std::fs::write(
        tmp.path().join("system.json"),
        serde_json::json!({ "commands": { "ownerDisplay": "feishu:oc_select_tests" } }).to_string(),
    )
    .expect("write system.json");
    let config_mgr = make_config_manager(&tmp);
    let _ = config_mgr.reload_section(ConfigSection::System, None);

    let session_mgr = make_session_manager();
    let gateway = make_gateway();

    // Subscribe the snapshot stream first, then run the real write path so
    // exactly one snapshot is buffered for this receiver. The event stream
    // is subscribed afterwards, so a newly created receiver starts at the
    // broadcast tail and only the Failed event injected below is seen.
    let mut snapshot_rx = config_mgr.subscribe_config_snapshots();
    config_mgr.update_section_cache(
        ConfigSection::Models,
        tmp.path().join("models.json"),
        serde_json::json!({ "version": "2.0" }),
    );
    let mut event_rx = config_mgr.subscribe_config_changes();
    config_mgr.notify_change(ConfigChangeEvent::Failed {
        section: ConfigSection::Channels,
        path: "channels.json".into(),
        error: "injected parse failure".to_string(),
    });

    let outcome = tokio::time::timeout(
        Duration::from_secs(2),
        handle_next_event(
            &mut event_rx,
            &config_mgr,
            &session_mgr,
            &gateway,
            &mut snapshot_rx,
        ),
    )
    .await
    .expect("Failed handling must complete instead of hanging on the owner notification");

    assert!(
        matches!(outcome, EventOutcome::Continue),
        "Failed event must be handled and report Continue, got {outcome:?}"
    );
    assert!(
        matches!(event_rx.try_recv(), Err(TryRecvError::Empty)),
        "Failed event must be consumed from the event stream"
    );
    assert!(
        snapshot_rx.try_recv().is_ok(),
        "Failed branch must leave the snapshot stream untouched (Reloaded would consume it)"
    );
}

// ---------------------------------------------------------------------------
// Error path — Lagged → Continue, then the retained event is handled
// ---------------------------------------------------------------------------

/// Error path: a lagged config-change stream is absorbed (→
/// [`EventOutcome::Continue`], snapshot stream untouched) and the event
/// the lag left behind is fully handled on the next call — the flattened
/// future neither exits nor wedges after a `Lagged` error.
///
/// A capacity-1 broadcaster plus 10 sends guarantees the `Lagged` error;
/// with capacity 1 the only retained message is the newest `Reloaded`,
/// so the second call must handle it end-to-end (it consumes the one
/// snapshot seeded before the lag). `probe_rx` and `snapshot_rx` are two
/// independent receivers of that same snapshot: probing consumes only the
/// probe's copy, leaving the handler's copy for the second call.
#[tokio::test]
async fn test_handle_next_event_lagged_continues_then_handles_retained_event() {
    let tmp = TempDir::new().unwrap();
    let config_mgr = make_config_manager(&tmp);
    let session_mgr = make_session_manager();
    let gateway = make_gateway();

    let mut probe_rx = config_mgr.subscribe_config_snapshots();
    let mut snapshot_rx = config_mgr.subscribe_config_snapshots();
    config_mgr.update_section_cache(
        ConfigSection::Models,
        tmp.path().join("models.json"),
        serde_json::json!({ "version": "2.0" }),
    );

    let broadcaster = ConfigChangeBroadcaster::with_capacity(1);
    let mut event_rx = broadcaster.subscribe();
    for i in 0..10 {
        broadcaster.send(ConfigChangeEvent::Reloaded {
            section: ConfigSection::Models,
            path: format!("models-{i}.json").into(),
        });
    }

    let lagged_outcome = tokio::time::timeout(
        Duration::from_secs(2),
        handle_next_event(
            &mut event_rx,
            &config_mgr,
            &session_mgr,
            &gateway,
            &mut snapshot_rx,
        ),
    )
    .await
    .expect("lag absorption must complete instead of parking");
    assert!(
        matches!(lagged_outcome, EventOutcome::Continue),
        "a lagged stream must be absorbed as Continue, got {lagged_outcome:?}"
    );
    assert!(
        probe_rx.try_recv().is_ok(),
        "the Lagged branch must leave the snapshot stream untouched"
    );

    let next_outcome = tokio::time::timeout(
        Duration::from_secs(2),
        handle_next_event(
            &mut event_rx,
            &config_mgr,
            &session_mgr,
            &gateway,
            &mut snapshot_rx,
        ),
    )
    .await
    .expect("the retained post-lag event must be handled instead of parking");
    assert!(
        matches!(next_outcome, EventOutcome::Continue),
        "the retained event must be handled and report Continue, got {next_outcome:?}"
    );
    assert!(
        matches!(snapshot_rx.try_recv(), Err(TryRecvError::Empty)),
        "the retained Reloaded event must be fully handled (its snapshot consumed)"
    );
}

// ---------------------------------------------------------------------------
// False-update contract — `send(false)` never stops the loop
// ---------------------------------------------------------------------------

/// False-update contract (issue #3220, documented invariant): a `false`
/// update on the shutdown watch channel must **not** stop the loop —
/// [`shutdown_exit_requested`] answers "no shutdown requested yet" and
/// the loop re-arms its single-level `select!`.
///
/// The loop-level half of the contract: the task survives the `false`
/// update, accepts further config events injected afterwards without
/// wedging, and still honors a later explicit `send(true)` with a bounded
/// clean exit. A regression that breaks out on `false` fails the first
/// liveness assert; a regression that stops re-arming `select!` (or
/// panics on the injected event) fails the bounded exit.
#[tokio::test]
async fn test_subscriber_false_update_keeps_loop_alive_and_serving() {
    let tmp = TempDir::new().unwrap();
    let config_mgr = make_config_manager(&tmp);
    let (shutdown_tx, subscriber) = spawn_test_subscriber(&config_mgr, make_session_manager());

    tokio::task::yield_now().await;
    assert!(
        !subscriber.is_finished(),
        "subscriber must be running before the false update"
    );

    // False update: changed() fires with value `false` → no exit.
    shutdown_tx
        .send(false)
        .expect("shutdown sender should still be open");
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    assert!(
        !subscriber.is_finished(),
        "a false update must not exit the subscriber loop (documented contract)"
    );

    // Subsequent events are still accepted: a bare Reloaded event (no
    // snapshot broadcast) parks the handler inside the loop's `select!`,
    // and the loop must neither exit nor panic on it.
    config_mgr.notify_change(ConfigChangeEvent::Reloaded {
        section: ConfigSection::Models,
        path: "models.json".into(),
    });
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    assert!(
        !subscriber.is_finished(),
        "the loop must still accept events after a false update"
    );

    // Real write path releases the parked handler (snapshot broadcast),
    // the loop re-arms, and an explicit shutdown must still end in a
    // bounded clean exit — proving the loop stayed serviceable
    // end-to-end after the `false` update.
    config_mgr.update_section_cache(
        ConfigSection::Models,
        tmp.path().join("models.json"),
        serde_json::json!({ "version": "2.0" }),
    );
    tokio::task::yield_now().await;
    shutdown_tx
        .send(true)
        .expect("shutdown sender should still be open");

    assert_subscriber_exits(subscriber, 2, "explicit shutdown after false update").await;
}

/// Decision-function half of the false-update contract: a `changed()`
/// observation whose value is still `false` (sender alive) makes
/// [`shutdown_exit_requested`] return `false`, i.e. the loop re-arms
/// instead of breaking — the exact defensive `else` arm documented on
/// that function (issue #3220 方案 1).
///
/// `watch::Sender::send` notifies even when the value is unchanged, so
/// `send(false)` really produces the `Ok` observation under test here.
#[tokio::test]
async fn test_shutdown_exit_requested_false_update_returns_false() {
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    shutdown_tx
        .send(false)
        .expect("shutdown sender should still be open");
    let changed = shutdown_rx.changed().await;
    assert!(
        changed.is_ok(),
        "a false update must still be observed as a change (Ok result)"
    );

    let exit_requested = shutdown_exit_requested(changed, &shutdown_rx);
    assert!(
        !exit_requested,
        "a false update with a live sender must not request exit, got {exit_requested}"
    );
}
