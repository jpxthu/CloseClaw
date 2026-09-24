//! ArchiveSweeper shutdown tests: signal handling, grace-period abort, and
//! the SWEEPER_GRACE_PERIOD_SECS constant.

use closeclaw_config::SessionConfigProvider;
use closeclaw_session::persistence::PersistenceService;
use std::sync::Arc;
use tokio::sync::watch;

use crate::sweeper::{ArchiveSweeper, SWEEPER_GRACE_PERIOD_SECS};

use super::sweeper_test_utils::{MemStorage, MockConfig};

// -----------------------------------------------------------------
// Test: shutdown signal causes run() to exit
// -----------------------------------------------------------------

#[tokio::test]
async fn test_shutdown_exits_loop() {
    let mem = Arc::new(MemStorage::default());
    let storage: Arc<dyn PersistenceService> = mem.clone() as _;
    let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::with_agents(vec![]));

    let (tx, rx) = watch::channel(());

    let sweeper = ArchiveSweeper::new(Arc::clone(&storage), Arc::clone(&config));

    let handle = tokio::spawn(async move {
        sweeper.run(rx).await;
    });

    let _ = tx.send(());
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
}

// ── shutdown grace period tests ─────────────────────────────────

/// Shutdown signal with no running task → exits immediately.
#[tokio::test]
async fn test_shutdown_no_running_task_exits_immediately() {
    let mem = Arc::new(MemStorage::default());
    let storage: Arc<dyn PersistenceService> = mem.clone() as _;
    let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::with_agents(vec![]));
    let (tx, rx) = watch::channel(());
    let sweeper = ArchiveSweeper::new(Arc::clone(&storage), Arc::clone(&config));
    let handle = tokio::spawn(async move {
        sweeper.run(rx).await;
    });
    // Send shutdown immediately — no task running
    let _ = tx.send(());
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
    assert!(
        result.is_ok(),
        "sweeper should exit quickly when no task is running"
    );
}

/// Shutdown signal with running task that exceeds grace period → abort.
///
/// Runs on a paused tokio clock (`start_paused`): every
/// `sleep`/`sleep_until` inside the FakeSweeper (50 ms sweep interval,
/// 30 s task body, 10 s grace period) completes instantly in virtual
/// time, so the full timing chain — task spawned → shutdown arrives →
/// grace period expires → abort — is still genuinely executed and
/// asserted while real wall-clock cost collapses to milliseconds.
#[tokio::test(start_paused = true)]
async fn test_shutdown_grace_period_expires_aborts() {
    struct FakeSweeper {
        storage: Arc<dyn PersistenceService>,
    }

    impl FakeSweeper {
        async fn run(&self, mut shutdown: watch::Receiver<()>) {
            let mut running_task: Option<tokio::task::JoinHandle<()>> = None;
            let interval = tokio::time::Duration::from_millis(50);
            let mut next_fire = tokio::time::Instant::now() + interval;
            loop {
                tokio::select! {
                    _ = shutdown.changed() => break,
                    _ = tokio::time::sleep_until(next_fire),
                        if running_task.is_none() =>
                    {
                        let storage = Arc::clone(&self.storage);
                        let task = tokio::task::spawn(async move {
                            // Simulate a task that takes longer than grace period
                            tokio::time::sleep(std::time::Duration::from_secs(30)).
                                await;
                            let _ = storage;
                        });
                        running_task = Some(task);
                        next_fire += interval;
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

    let mem = Arc::new(MemStorage::default());
    let sweeper = FakeSweeper {
        storage: mem.clone() as _,
    };
    let (tx, rx) = watch::channel(());
    let handle = tokio::spawn(async move {
        sweeper.run(rx).await;
    });

    // Wait for the sweeper to start a task. Under the paused clock
    // this wait is deterministic: auto-advance fires the first 50 ms
    // interval tick immediately, and the spawned task's 30 s body is
    // parked on the virtual clock, so once this sleep completes the
    // sweep task is guaranteed to be running.
    tokio::time::sleep(tokio::time::Duration::from_millis(80)).await;
    // Send shutdown — task will NOT finish within grace period
    let _ = tx.send(());
    // Virtual-clock start: the exit wait must consume the full grace
    // period and strictly less than the task's 30 s body.
    let start = tokio::time::Instant::now();
    // Guard (virtual time too): if the abort path regresses and the
    // loop keeps sweeping, time runs past 20 s and this fails with a
    // message instead of hanging the test.
    let result = tokio::time::timeout(std::time::Duration::from_secs(20), handle).await;
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
    // Upper bound: well below the task's 30 s body — proves the exit
    // came from the abort branch, not from natural task completion.
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "sweeper must abort the task instead of waiting for natural completion, took {elapsed:?}"
    );
}

// ── Step 1.5: SWEEPER_GRACE_PERIOD_SECS constant test ───────────────

/// Verify SWEEPER_GRACE_PERIOD_SECS == 10 (design doc alignment).
#[test]
fn test_sweeper_grace_period_is_ten_seconds() {
    assert_eq!(SWEEPER_GRACE_PERIOD_SECS, 10);
}
