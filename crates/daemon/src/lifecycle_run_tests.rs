//! Direct drive tests for the `Daemon::run()` main loop (issue #4065).
//!
//! First tests to reach the main `select!` loop itself — previously no
//! test drove `run()` (tests.rs starts the daemon but deliberately never
//! enters the loop). Each test builds the full daemon via the
//! `start_with_pid_file_path` fixture, swaps in injected restart/admin
//! channels, and observes the loop deterministically through the
//! restart-state watch channel before delivering a real SIGTERM via
//! `kill_self`:
//!
//! - Normal path: the loop consumes a restart probe (handshake proving
//!   the loop is running), then the shutdown signal drives the full
//!   Phase 0-7 exit and `run()` returns `Ok`.
//! - State transition: an admin restart command (`force = false`) flips
//!   the restart state machine `Pending → Idle` inside the loop.
//! - Boundary: a restart channel closed before `run()` (recv → `None`)
//!   disables its branch; the loop survives and shuts down cleanly.
//!
//! All three are `#[serial_test::serial]` (process-wide signal broadcast,
//! STANDARDS §7; precedent aa9f91f9). Watchdog note: the loop's restart
//! branch spawns the watchdog task, but the watchdog subscribes to the
//! state watch AFTER `Pending` was set, so its `changed()` waits for a
//! later transition — no gateway restart executes during these tests.

use crate::gateway_restart::RestartState;
use crate::test_helpers::{
    isolated_pid_file, kill_self, setup_agents_json, signal_step, TestShutdownSignal,
};
use crate::Daemon;
use tempfile::TempDir;
use tokio::sync::{mpsc, watch};

/// Start a full daemon against a fresh TempDir with the isolated PID
/// file (shared `test_helpers` fixtures, ~1s).
async fn start_run_test_daemon() -> anyhow::Result<(TempDir, Daemon)> {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    setup_agents_json(temp_dir.path()).expect("setup agents.json");
    let daemon = Daemon::start_with_pid_file_path(
        temp_dir.path().to_str().unwrap(),
        Some(isolated_pid_file(temp_dir.path())),
    )
    .await?;
    Ok((temp_dir, daemon))
}

/// Shared SIGTERM teardown of all three tests: deliver the real
/// graceful signal, await `run()` under the signal-step timeout, and
/// assert the graceful `Ok` return. The per-test diagnostics are
/// parameters: `step_what` labels the shutdown timeout, `join_expect`
/// the join failure, and `ok_context` completes the
/// `run() must return Ok {ok_context}` assertion.
async fn sigterm_and_join_run(
    step_what: &str,
    mut run_task: tokio::task::JoinHandle<anyhow::Result<()>>,
    join_expect: &str,
    ok_context: &str,
) {
    kill_self(TestShutdownSignal::Sigterm);
    let result = signal_step(step_what, &mut run_task)
        .await
        .expect(join_expect);
    assert!(
        result.is_ok(),
        "run() must return Ok {ok_context}: {result:?}"
    );
}

// ── 维度① 正常路径 ────────────────────────────────────────────────────────

/// The loop consumes a restart probe (proving it is running and its
/// shutdown-signal subscription is registered), then a real SIGTERM
/// drives the full Phase 0-7 exit: `run()` returns `Ok` within the
/// signal-step timeout. The restart state stays `Pending` — the spawned
/// watchdog never fires because it subscribes after `Pending` was set.
#[serial_test::serial]
#[tokio::test]
async fn test_run_loop_exits_gracefully_on_sigterm() {
    let (_temp_dir, mut daemon) = start_run_test_daemon().await.expect("daemon must start");

    // Inject a restart channel whose sender the test holds; the original
    // config-watcher receiver is dropped (nothing sends on it mid-test).
    let (restart_tx, restart_rx) = mpsc::channel::<String>(4);
    daemon.restart_rx = Some(restart_rx);

    let mut state_rx = daemon.restart_state.subscribe();
    assert_eq!(
        *state_rx.borrow(),
        RestartState::Idle,
        "precondition: fresh daemon starts Idle"
    );

    let run_task = tokio::spawn(async move { daemon.run().await });

    // Handshake: one restart message through the loop's restart branch
    // flips Idle → Pending. Observing it proves the select! loop is
    // running (shutdown signals already subscribed) before we signal.
    restart_tx
        .send("config reload probe".to_string())
        .await
        .expect("restart channel open");
    signal_step(
        "run() loop did not consume the restart probe within 10s",
        state_rx.changed(),
    )
    .await
    .expect("restart-state watch must stay open while run() lives");
    assert!(
        matches!(*state_rx.borrow(), RestartState::Pending { .. }),
        "probe must leave the restart state Pending, got {:?}",
        state_rx.borrow().clone()
    );

    // Loop proven running — deliver the real graceful shutdown signal.
    sigterm_and_join_run(
        "run() did not complete the full shutdown within 10s",
        run_task,
        "run task must not be aborted",
        "after graceful shutdown",
    )
    .await;
    // No restart ever executed: the state must still be Pending (the
    // watchdog never fired — its changed() is still waiting).
    assert!(
        matches!(*state_rx.borrow(), RestartState::Pending { .. }),
        "state must remain Pending (no restart executed), got {:?}",
        state_rx.borrow().clone()
    );
}

// ── 维度② 状态转换 + 共享 admin-cancel 握手 ──────────────────────────────

/// Seed the restart state machine to `Pending` (direct
/// `request_gateway_restart` call — deliberately WITHOUT spawning the
/// watchdog), inject an admin restart command channel, spawn `run()`,
/// then drive a real cancel command (`force = false`) through the loop's
/// admin restart branch.
///
/// Returns only after the loop processed the cancel: the state watch
/// reports `Idle`, observable only from `cancel_pending_restart()`
/// running inside the select! loop. The caller therefore knows `run()`
/// is live and subscribed to shutdown signals before delivering its own
/// signal. The admin sender is returned so the channel stays open.
#[allow(clippy::type_complexity)]
async fn spawn_run_and_cancel_pending(
    mut daemon: Daemon,
) -> (
    tokio::task::JoinHandle<anyhow::Result<()>>,
    mpsc::Sender<bool>,
    watch::Receiver<RestartState>,
) {
    assert!(
        daemon.request_gateway_restart(vec!["seed change".to_string()]),
        "precondition: request from Idle must seed Pending (should_spawn=true)"
    );
    let (admin_tx, admin_rx) = mpsc::channel::<bool>(2);
    daemon.admin_restart_rx = Some(admin_rx);
    let mut state_rx = daemon.restart_state.subscribe();
    assert!(
        matches!(*state_rx.borrow(), RestartState::Pending { .. }),
        "precondition: state must be Pending before run(), got {:?}",
        state_rx.borrow().clone()
    );

    let run_task = tokio::spawn(async move { daemon.run().await });

    // Real message into the admin restart branch: force=false → cancel.
    admin_tx.send(false).await.expect("admin channel open");
    signal_step(
        "run() loop did not process the admin cancel within 10s",
        state_rx.changed(),
    )
    .await
    .expect("restart-state watch must stay open while run() lives");
    assert_eq!(
        state_rx.borrow().clone(),
        RestartState::Idle,
        "admin cancel (force=false) must transition Pending → Idle"
    );
    (run_task, admin_tx, state_rx)
}

/// A real admin restart command (`force = false`) flips the restart
/// state machine `Pending → Idle` inside the loop (asserted by the
/// handshake helper), and the loop keeps running until the shutdown
/// signal exits it cleanly.
#[serial_test::serial]
#[tokio::test]
async fn test_run_loop_admin_cancel_transitions_pending_to_idle() {
    let (_temp_dir, daemon) = start_run_test_daemon().await.expect("daemon must start");

    let (run_task, _admin_tx, state_rx) = spawn_run_and_cancel_pending(daemon).await;

    sigterm_and_join_run(
        "run() did not complete the full shutdown within 10s",
        run_task,
        "run task must not be aborted",
        "after cancel + shutdown",
    )
    .await;
    // State stayed Idle — the cancel was not undone by a later restart.
    assert_eq!(
        state_rx.borrow().clone(),
        RestartState::Idle,
        "state must remain Idle through shutdown"
    );
}

// ── 维度③ 边界：重启通道闭合 ─────────────────────────────────────────────

/// The restart channel is closed before `run()` starts: in loop round 1
/// the biased select! polls shutdown (still pending) and then the
/// restart branch, whose `recv` returns `None` — the branch disables
/// itself instead of panicking, and as the first ready branch it
/// short-circuits the round. Round 2 then skips the disabled restart
/// branch and polls the admin branch, whose cancel command flips
/// Pending → Idle — an observation impossible before the round-1 `None`
/// consumption. The loop then runs on until the shutdown signal exits
/// it with `Ok`.
#[serial_test::serial]
#[tokio::test]
async fn test_run_loop_survives_closed_restart_channel_until_shutdown() {
    let (_temp_dir, mut daemon) = start_run_test_daemon().await.expect("daemon must start");

    let (restart_tx, restart_rx) = mpsc::channel::<String>(4);
    drop(restart_tx);
    daemon.restart_rx = Some(restart_rx);

    let (run_task, _admin_tx, state_rx) = spawn_run_and_cancel_pending(daemon).await;

    sigterm_and_join_run(
        "run() did not complete the full shutdown within 10s",
        run_task,
        "run task must not be aborted (no panic on closed channel)",
        "despite the closed restart channel",
    )
    .await;
    assert_eq!(
        state_rx.borrow().clone(),
        RestartState::Idle,
        "state must remain Idle through shutdown"
    );
}
