//! Direct drive tests for the `Daemon::run()` main loop (issues #4065, #4067).
//!
//! First tests to reach the main `select!` loop itself — previously no
//! test drove `run()` (tests.rs starts the daemon but deliberately never
//! enters the loop). Each test builds the full daemon via the
//! `start_with_pid_file_path` fixture, swaps in injected restart/admin
//! channels, and observes the loop deterministically through the
//! restart-state watch channel before delivering a real SIGTERM via
//! `kill_self`:
//!
//! - Pending × SIGTERM: the loop consumes a restart probe (handshake
//!   proving the loop is running), then the shutdown signal must drive
//!   the full Phase 0-7 exit in **forceful** mode (restart.md §架构
//!   「无窗口兜底」) despite a held active operation — the busy-count
//!   trap: graceful would sit in the Phase 1 drain until the 30s drain
//!   timeout, far beyond the 10s signal-step bound. `run()` returns
//!   `Ok` and the state stays `Pending`.
//! - Idle guard × SIGTERM: with a held active operation, the shutdown
//!   must initiate **graceful** (`ShuttingDown`) and keep waiting for
//!   the operation — forceful is reserved for Pending/Executing
//!   restarts; `run()` returns `Ok` once it completes, state `Idle`.
//! - State transition: an admin restart command (`force = false`) flips
//!   the restart state machine `Pending → Idle` inside the loop.
//! - Boundary: a restart channel closed before `run()` (recv → `None`)
//!   disables its branch; the loop survives and shuts down cleanly.
//!
//! All are `#[serial_test::serial]` (process-wide signal broadcast,
//! STANDARDS §7; precedent aa9f91f9). Watchdog note: the loop's restart
//! branch spawns the watchdog task, but the watchdog subscribes to the
//! state watch AFTER `Pending` was set, so its `changed()` waits for a
//! later transition — no gateway restart executes during these tests.
//!
//! Executing × shutdown (the third mode-determination state) lives in
//! `gateway_restart_tests.rs` at unit level: manual seeding needs the
//! module-private watch sender, and a full `run()` × Executing × SIGTERM
//! drive is not safely handshakable — `execute_gateway_restart` awaits
//! inside the select arm (issue #4069: a signal sent mid-rebuild is
//! buffered and processed at Idle), and no loop branch proves
//! consumption while preserving the `Executing` state.

use crate::gateway_restart::RestartState;
use crate::shutdown::ShutdownState;
use crate::test_helpers::{
    isolated_pid_file, kill_self, setup_agents_json, signal_step, TestShutdownSignal,
};
use crate::Daemon;
use std::sync::Arc;
use std::time::Duration;
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

/// Shared SIGTERM teardown of the tests that deliver the signal last:
/// deliver the real signal, await `run()` under the signal-step timeout,
/// and assert the graceful `Ok` return (the shutdown *mode* — forceful
/// or graceful — is decided by the restart state, not by the signal).
/// The per-test diagnostics are parameters: `step_what` labels the
/// shutdown timeout, `join_expect` the join failure, and `ok_context`
/// completes the `run() must return Ok {ok_context}` assertion.
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

// ── 维度① Pending × SIGTERM：forceful 语义 ────────────────────────────────

/// A pending restart + real SIGTERM → shutdown starts in **forceful**
/// mode (restart.md §架构「无窗口兜底」; shutdown.md §双模关闭 / §升级路径 /
/// §数据流 Phase 0). Behavioral discriminator — the busy-count trap: an
/// active operation is held for the whole test (nothing in the idle
/// daemon decrements it), so a graceful shutdown would sit in the
/// Phase 1 drain until the 30s drain timeout — the 10s signal-step
/// bound fails first — while the forceful path skips the drain wait
/// and completes Phase 0-7 within the step. `run()` returns `Ok`; the
/// state stays `Pending`: the shutdown aborts the pending restart by
/// exiting the loop before the watchdog could fire (the state itself
/// is intentionally left untouched).
#[serial_test::serial]
#[tokio::test]
async fn test_run_loop_pending_restart_sigterm_starts_forceful_shutdown() {
    let (_temp_dir, mut daemon) = start_run_test_daemon().await.expect("daemon must start");

    // Inject a restart channel whose sender the test holds; the original
    // config-watcher receiver is dropped (nothing sends on it mid-test).
    let (restart_tx, restart_rx) = mpsc::channel::<String>(4);
    daemon.restart_rx = Some(restart_rx);

    // Busy-count trap: the operation stays in flight through the whole
    // shutdown — only the forceful skip-drain path can finish in time.
    daemon.shutdown.increment_busy();

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

    // Loop proven running with a pending restart — deliver the real
    // shutdown signal; forceful semantics must complete within the step
    // (graceful would block on the 30s drain and lose the race).
    sigterm_and_join_run(
        "run() did not complete the forceful shutdown within 10s",
        run_task,
        "run task must not be aborted",
        "after forceful shutdown (pending restart)",
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

// ── 维度② Idle × SIGTERM：graceful 守护 ──────────────────────────────────

/// Idle (no pending restart) + real SIGTERM → graceful semantics are
/// kept: the shutdown initiates as `ShuttingDown` and keeps **waiting**
/// for the held active operation (busy-count trap) instead of skipping
/// the drain — forceful is reserved for Pending/Executing restarts
/// (restart.md §架构「无窗口兜底」), a guard against accidental
/// escalation. Observation is state-based, not timing-based: a forceful
/// regression races Running → ForcefulShuttingDown → Stopped in one
/// synchronous stretch (drain skipped despite the busy operation), so
/// it can never present a stable `ShuttingDown` while busy ≥ 1 — every
/// wrong reading fails the loop below. After the operation completes,
/// the drain finishes and `run()` returns `Ok` with the state `Idle`.
///
/// The admin-cancel handshake (Pending → Idle observed from inside the
/// loop) both restores the `Idle` precondition and proves the loop is
/// running with its shutdown-signal subscription registered before
/// `kill_self` — signaling earlier would hit the default SIGTERM
/// disposition and kill the test process.
#[serial_test::serial]
#[tokio::test]
async fn test_run_loop_idle_sigterm_keeps_graceful_drain_wait() {
    let (_temp_dir, daemon) = start_run_test_daemon().await.expect("daemon must start");

    // Active operation in flight while SIGTERM arrives (busy-count trap).
    let shutdown = Arc::clone(&daemon.shutdown);
    shutdown.increment_busy();

    // Seed Pending before run(), then cancel from inside the loop — the
    // helper returns only after the cancel is observed: the loop is live,
    // subscribed, and the state is back at Idle for the graceful guard.
    let (mut run_task, _admin_tx, state_rx) = spawn_run_and_cancel_pending(daemon).await;

    kill_self(TestShutdownSignal::Sigterm);

    // Bounded observation (1s cap, STANDARDS §6): graceful must engage
    // (`ShuttingDown`) and stay put while the operation is in flight.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        match shutdown.state() {
            ShutdownState::Running => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "shutdown must initiate (ShuttingDown) within 1s of SIGTERM"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            ShutdownState::ShuttingDown => break,
            other => panic!(
                "graceful shutdown must wait for the active operation — no \
                 forceful escalation without a pending restart; got {other:?} \
                 while busy_count={}",
                shutdown.busy_count()
            ),
        }
    }
    assert_eq!(
        shutdown.busy_count(),
        1,
        "the active operation must still be in flight — the graceful drain \
         is waiting for it (forceful would have skipped the wait)"
    );

    // Operation completes → drain unblocks → full Phase 0-7 exit.
    shutdown.decrement_busy();
    let result = signal_step(
        "run() did not complete the graceful shutdown within 10s after the \
         active operation finished",
        &mut run_task,
    )
    .await
    .expect("run task must not be aborted");
    assert!(
        result.is_ok(),
        "run() must return Ok after the graceful shutdown: {result:?}"
    );

    // No restart machinery engaged: the state stayed Idle throughout.
    assert_eq!(
        state_rx.borrow().clone(),
        RestartState::Idle,
        "state must remain Idle through shutdown"
    );
}

// ── 维度③ 状态转换 + 共享 admin-cancel 握手 ──────────────────────────────

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

// ── 维度④ 边界：重启通道闭合 ─────────────────────────────────────────────

/// The restart channel is closed before `run()` starts: in loop round 1
/// the biased select! polls shutdown (still pending) and then the
/// restart branch, whose `recv` returns `None` — the branch disables
/// itself instead of panicking, and as the first ready branch it
/// short-circuits the round. Round 2 then skips the disabled restart
/// branch and polls the admin branch, whose cancel command flips
/// Pending → Idle — an observation impossible before the round-1 `None`
/// consumption. The loop then runs on until the shutdown signal exits
/// it with `Ok` (graceful — channel closures are not shutdown signals,
/// so the forceful mode-determination does not apply).
///
/// Coverage note (issue #4067): the *signal*-stream-closed fallback
/// (run.rs `next_signal() → None`) keeps graceful by design but has no
/// black-box trigger — the platform `next_signal` maps every recv
/// outcome of its SIGINT/SIGTERM streams to `Some(kind)`, so that arm
/// is defensively unreachable today and stays uncovered here.
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
