//! Periodic media cleanup task.
//!
//! Schedules recurring cleanup of expired media files by calling a
//! caller-provided closure that returns the current `retention_days`.
//! When `retention_days` is 0, cleanup is skipped for that cycle
//! (supporting hot-reload of config changes).

use std::sync::Arc;

/// Callback that returns the current retention period in days.
///
/// Returning `0` disables cleanup for the current cycle (supports
/// hot-reload: config change takes effect on the next scan).
pub type RetentionProvider = Arc<dyn Fn() -> u64 + Send + Sync>;

/// Callback that performs the actual cleanup.
///
/// Returns the number of files removed, or an error.
pub type CleanupFn =
    Arc<dyn Fn(u64) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> + Send + Sync>;

/// Handle to a running [`MediaCleanupTask`].
///
/// Dropping this handle does **not** stop the task — the background
/// task continues running until the process exits. To stop it early,
/// use [`MediaCleanupTask::shutdown`].
#[derive(Debug)]
pub struct MediaCleanupHandle {
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl MediaCleanupHandle {
    /// Signal the cleanup task to stop after its current cycle.
    pub fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for MediaCleanupHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Start a periodic media cleanup task.
///
/// # Arguments
///
/// * `interval` — Time between cleanup cycles (e.g. `Duration::from_secs(3600)`).
/// * `retention_provider` — Returns the current retention period in days.
/// * `cleanup_fn` — Performs the actual file cleanup.
///
/// # Returns
///
/// A [`MediaCleanupHandle`] that can be used to shut down the task.
pub fn start_media_cleanup(
    interval: std::time::Duration,
    retention_provider: RetentionProvider,
    cleanup_fn: CleanupFn,
) -> MediaCleanupHandle {
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    tokio::spawn(run_cleanup_loop(
        interval,
        retention_provider,
        cleanup_fn,
        shutdown_rx,
    ));

    MediaCleanupHandle {
        shutdown_tx: Some(shutdown_tx),
    }
}

/// The main cleanup loop.
async fn run_cleanup_loop(
    interval: std::time::Duration,
    retention_provider: RetentionProvider,
    cleanup_fn: CleanupFn,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => {
                tracing::info!("media cleanup task shutting down");
                break;
            }
            _ = tokio::time::sleep(interval) => {}
        }

        let retention_days = retention_provider();
        if retention_days == 0 {
            tracing::debug!("media cleanup: retention_days=0, skipping");
            continue;
        }

        match tokio::task::spawn_blocking({
            let cleanup_fn = Arc::clone(&cleanup_fn);
            move || cleanup_fn(retention_days)
        })
        .await
        {
            Ok(Ok(removed)) => {
                if removed > 0 {
                    tracing::info!(
                        removed = removed,
                        retention_days = retention_days,
                        "media cleanup: removed expired files"
                    );
                } else {
                    tracing::debug!(
                        retention_days = retention_days,
                        "media cleanup: no expired files"
                    );
                }
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    error = %e,
                    "media cleanup: cleanup function returned error"
                );
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "media cleanup: blocking task panicked"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Helper: create a retention provider that returns a fixed value.
    fn fixed_retention(days: u64) -> RetentionProvider {
        Arc::new(move || days)
    }

    /// Helper: create a cleanup counter that records invocations.
    fn counting_cleanup() -> (CleanupFn, Arc<AtomicU64>) {
        let count = Arc::new(AtomicU64::new(0));
        let c = count.clone();
        let f: CleanupFn = Arc::new(move |retention| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(retention) // return retention as "removed" count for assertion
        });
        (f, count)
    }

    #[tokio::test]
    async fn cleanup_runs_after_interval() {
        let (cleanup_fn, count) = counting_cleanup();
        let retention = fixed_retention(7);

        let handle =
            start_media_cleanup(std::time::Duration::from_millis(50), retention, cleanup_fn);

        // Wait for at least one cycle.
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        handle.shutdown();

        assert!(
            count.load(Ordering::SeqCst) >= 1,
            "cleanup should have run at least once"
        );
    }

    #[tokio::test]
    async fn retention_zero_skips_cleanup() {
        let (cleanup_fn, count) = counting_cleanup();
        let retention = fixed_retention(0);

        let handle =
            start_media_cleanup(std::time::Duration::from_millis(50), retention, cleanup_fn);

        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        handle.shutdown();

        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "cleanup should not run when retention_days=0"
        );
    }

    #[tokio::test]
    async fn shutdown_stops_cleanup() {
        let (cleanup_fn, count) = counting_cleanup();
        let retention = fixed_retention(7);

        let handle =
            start_media_cleanup(std::time::Duration::from_millis(50), retention, cleanup_fn);

        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        let before = count.load(Ordering::SeqCst);

        handle.shutdown();

        // Wait to confirm no more runs.
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        let after = count.load(Ordering::SeqCst);

        // After shutdown, at most one more cycle may have been in flight.
        assert!(
            after <= before + 1,
            "cleanup should stop after shutdown: before={before}, after={after}"
        );
    }

    #[tokio::test]
    async fn cleanup_error_does_not_panic() {
        let cleanup_fn: CleanupFn = Arc::new(|_| Err("simulated error".into()));
        let retention = fixed_retention(7);

        let handle =
            start_media_cleanup(std::time::Duration::from_millis(50), retention, cleanup_fn);

        // Should not panic even with errors.
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        handle.shutdown();
    }

    #[tokio::test]
    async fn hot_reload_retention_change() {
        let count = Arc::new(AtomicU64::new(0));
        let c = count.clone();
        let cleanup_fn: CleanupFn = Arc::new(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(0)
        });

        // Start with retention=0 (skip).
        let retention_days = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let r = retention_days.clone();
        let retention_provider: RetentionProvider = Arc::new(move || r.load(Ordering::SeqCst));

        let handle = start_media_cleanup(
            std::time::Duration::from_millis(50),
            retention_provider,
            cleanup_fn,
        );

        // Wait — should skip.
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        assert_eq!(count.load(Ordering::SeqCst), 0, "should skip with 0");

        // Hot-reload: change to 7 days.
        retention_days.store(7, Ordering::SeqCst);

        // Wait for next cycle.
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        handle.shutdown();

        assert!(
            count.load(Ordering::SeqCst) >= 1,
            "should run after hot-reload to retention=7"
        );
    }
}
