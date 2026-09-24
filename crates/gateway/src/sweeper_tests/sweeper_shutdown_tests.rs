//! ArchiveSweeper shutdown tests: signal handling, grace-period abort, and
//! the SWEEPER_GRACE_PERIOD_SECS constant.

use closeclaw_session::persistence::PersistenceService;
use std::sync::Arc;
use tokio::sync::watch;

use crate::sweeper::SWEEPER_GRACE_PERIOD_SECS;

use super::sweeper_test_utils::{sweeper_with_agents, MemStorage};

// -----------------------------------------------------------------
// Test: shutdown signal causes run() to exit
// -----------------------------------------------------------------

#[tokio::test]
async fn test_shutdown_exits_loop() {
    let (_mem, sweeper) = sweeper_with_agents(vec![]);

    let (tx, rx) = watch::channel(());

    let handle = tokio::spawn(async move {
        sweeper.run(rx).await;
    });

    let _ = tx.send(());
    // State transition: after the shutdown signal, run() must return
    // within this 1 s guard — a hang fails here instead of passing
    // vacuously (STANDARDS §6: no >1 s waits in unit tests).
    let result = tokio::time::timeout(tokio::time::Duration::from_secs(1), handle).await;
    assert!(
        result.is_ok(),
        "run() must exit within the 1 s guard after the shutdown signal, \
         guard expired without return"
    );
    // Error path: the sweeper task exits normally, without panicking.
    let join_result = result.expect("timeout is Ok, per assertion above");
    assert!(
        join_result.is_ok(),
        "sweeper task must not panic while exiting"
    );
}

// ── shutdown grace period tests ─────────────────────────────────

/// Shutdown signal with no running task → exits immediately.
#[tokio::test]
async fn test_shutdown_no_running_task_exits_immediately() {
    let (_mem, sweeper) = sweeper_with_agents(vec![]);
    let (tx, rx) = watch::channel(());
    let handle = tokio::spawn(async move {
        sweeper.run(rx).await;
    });
    // Send shutdown immediately — no task running
    let _ = tx.send(());
    let result = tokio::time::timeout(tokio::time::Duration::from_secs(1), handle).await;
    assert!(
        result.is_ok(),
        "sweeper should exit within the 1 s guard when no task is running, \
         guard expired without return"
    );
}

// Timing constants for the grace-abort test below (virtual time),
// one definition per concept. Invariant `fake_task_body > exit_guard >= grace`
// (30 s > 20 s >= 10 s, grace = SWEEPER_GRACE_PERIOD_SECS):
// - fake_task_body > grace: the fake task outlives the grace period, so
//   shutdown can only exit via the abort branch;
// - exit_guard >= grace: the grace-bounded exit wait fits inside the
//   exit-guard window, i.e. `elapsed ∈ [grace, exit_guard)` is achievable;
// - exit_guard < fake_task_body: an abort regression to natural completion
//   trips EXIT_GUARD before FAKE_TASK_BODY, failing `result.is_ok()` cleanly.
const FAKE_TASK_BODY: tokio::time::Duration = tokio::time::Duration::from_secs(30);
const EXIT_GUARD: tokio::time::Duration = tokio::time::Duration::from_secs(20);
/// Scan tick of [`FakeSweeper::run`] (virtual time under `start_paused`).
const SWEEP_INTERVAL: tokio::time::Duration = tokio::time::Duration::from_millis(50);

struct FakeSweeper {
    storage: Arc<dyn PersistenceService>,
}

impl FakeSweeper {
    // Cross-reference: this run() is a simplified inline mirror of the
    // production `ArchiveSweeper::run` and `wait_grace_period`
    // (crates/gateway/src/sweeper.rs) — the same select-loop structure
    // and grace-abort semantics are replicated here with a fake task.
    // Simplified: it omits the production `next_fire` drift correction
    // (the `Instant::now() > next_fire + interval` reset), keeping only
    // `next_fire += SWEEP_INTERVAL`.
    // When that production logic evolves, update this fake in
    // lockstep to prevent semantic drift.
    async fn run(&self, mut shutdown: watch::Receiver<()>) {
        let mut running_task: Option<tokio::task::JoinHandle<()>> = None;
        let mut next_fire = tokio::time::Instant::now() + SWEEP_INTERVAL;
        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                _ = tokio::time::sleep_until(next_fire),
                    if running_task.is_none() =>
                {
                    let storage = Arc::clone(&self.storage);
                    let task = tokio::task::spawn(async move {
                        // Simulate a task that takes longer than grace period
                        tokio::time::sleep(FAKE_TASK_BODY).await;
                        let _ = storage;
                    });
                    running_task = Some(task);
                    next_fire += SWEEP_INTERVAL;
                }
                result = async {
                    match running_task.as_mut() {
                        Some(t) => t.await,
                        None => std::future::pending().await,
                    }
                } => {
                    running_task = None;
                    if result.is_err() {
                        tracing::error!("task panicked");
                    }
                }
            }
        }
        if let Some(mut task) = running_task {
            let grace = tokio::time::Duration::from_secs(SWEEPER_GRACE_PERIOD_SECS);
            tokio::select! {
                result = &mut task => {
                    let _ = result;
                }
                _ = tokio::time::sleep(grace) => {
                    task.abort();
                    tracing::warn!("grace period expired, aborting");
                }
            }
        }
    }
}

/// Shutdown signal with running task that exceeds grace period → abort.
///
/// Runs on a paused tokio clock (`start_paused`): every
/// `sleep`/`sleep_until` inside the FakeSweeper (`SWEEP_INTERVAL` tick,
/// `FAKE_TASK_BODY` task, 10 s grace period) completes instantly in
/// virtual time, so the full timing chain — task spawned → shutdown arrives →
/// grace period expires → abort — is still genuinely executed and
/// asserted while real wall-clock cost collapses to milliseconds.
#[tokio::test(start_paused = true)]
async fn test_shutdown_grace_period_expires_aborts() {
    let mem = Arc::new(MemStorage::default());
    let sweeper = FakeSweeper {
        storage: mem.clone() as _,
    };
    let (tx, rx) = watch::channel(());
    let handle = tokio::spawn(async move {
        sweeper.run(rx).await;
    });

    // Wait for the sweeper to start a task. Under the paused clock
    // this wait is deterministic: auto-advance fires the first
    // SWEEP_INTERVAL tick immediately, and the spawned task's
    // FAKE_TASK_BODY sleep is parked on the virtual clock, so once
    // this sleep (SWEEP_INTERVAL + 30 ms slack) completes the sweep
    // task is guaranteed to be running.
    tokio::time::sleep(SWEEP_INTERVAL + tokio::time::Duration::from_millis(30)).await;
    // Send shutdown — task will NOT finish within grace period
    let _ = tx.send(());
    // Virtual-clock start: the exit wait must land in [grace, EXIT_GUARD)
    // — full grace consumed, exit strictly before exit-guard expiry.
    let start = tokio::time::Instant::now();
    // Exit guard (virtual time too): if the abort path regresses to
    // waiting for natural completion (FAKE_TASK_BODY > EXIT_GUARD, so
    // EXIT_GUARD fires first) or the loop never exits, this fails the
    // `result.is_ok()` assertion with a message instead of hanging
    // the test. Excluding natural task completion is established by
    // this exit-guard + the `is_ok()` chain, NOT by the upper bound below.
    let result = tokio::time::timeout(EXIT_GUARD, handle).await;
    let elapsed = start.elapsed();
    assert!(
        result.is_ok(),
        "sweeper should exit after grace period abort"
    );
    // Lower bound: the sweeper must actually wait out the entire
    // grace period before aborting (never vacuously early).
    let grace = tokio::time::Duration::from_secs(SWEEPER_GRACE_PERIOD_SECS);
    assert!(
        elapsed >= grace,
        "sweeper must wait out the full grace period before aborting, took {elapsed:?}"
    );
    // Upper bound, shared with the exit-guard timeout: exit strictly
    // before EXIT_GUARD — the abort fired inside the exit-guard window,
    // never after it.
    assert!(
        elapsed < EXIT_GUARD,
        "sweeper must abort before the exit-guard expires, took {elapsed:?}"
    );
}

// ── SWEEPER_GRACE_PERIOD_SECS constant test ────────────────────────

/// Verify SWEEPER_GRACE_PERIOD_SECS == 10 (design doc alignment).
#[test]
fn test_sweeper_grace_period_is_ten_seconds() {
    assert_eq!(SWEEPER_GRACE_PERIOD_SECS, 10);
}
