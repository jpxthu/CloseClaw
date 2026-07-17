//! Announce Sweeper — background task for spawn silent-failure protection.
//!
//! Periodically scans for run-mode child sessions that have completed
//! (three-dimensional execution state all zeroed) but whose announce
//! has not yet been delivered to the parent. This is the second layer
//! of the spawn silent-failure defense described in
//! `docs/design/session/run-health.md` §Spawn 静默失败防护.
//!
//! The sweeper runs at a fixed 60-second interval (per design doc)
//! and is spawned by the daemon at startup alongside the
//! `ArchiveSweeper`. It is shut down gracefully via a
//! `tokio::sync::watch` channel.
//!
//! This module lives in `closeclaw-session` (not `closeclaw-gateway`)
//! per the design doc requirement that Run Health owns the sweeper.
//! Gateway-layer specifics are injected via the [`AnnounceSweepTarget`]
//! trait.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::watch;
use tokio::time::Instant;
use tracing::{error, info, warn};

use closeclaw_tasks::NotificationPriority;

/// Fixed scan interval in seconds (design doc specifies 60s).
const ANNOUNCE_SWEEP_INTERVAL_SECS: u64 = 60;

/// Grace period (in seconds) to wait for a running sweep to finish
/// before forcibly aborting it on shutdown.
const ANNOUNCE_SWEEP_GRACE_PERIOD_SECS: u64 = 5;

/// Trait abstracting the gateway-layer operations that
/// [`AnnounceSweeper`] needs. Implemented by `SessionManager`
/// in the gateway crate, allowing the sweeper to live in the
/// session crate without a reverse dependency.
#[async_trait]
pub trait AnnounceSweepTarget: Send + Sync {
    /// Get all run-mode child sessions as `(child_id, parent_id)` pairs.
    async fn get_run_mode_children(&self) -> Vec<(String, String)>;

    /// Check whether a child session has been removed from the
    /// spawn tree (i.e. announce was already delivered).
    async fn is_child_removed(&self, child_id: &str) -> bool;

    /// Check whether a session's three-dimensional execution
    /// status is `Idle`.
    async fn is_session_idle(&self, session_id: &str) -> bool;

    /// Push an announce event from a completed child to its parent.
    async fn try_push_announce(&self, session_id: &str, priority: NotificationPriority);
}

/// Background sweeper that ensures completion announces from run-mode
/// child sessions reach their parent even if the normal即时路径
/// missed the delivery.
pub struct AnnounceSweeper {
    target: Arc<dyn AnnounceSweepTarget>,
}

impl AnnounceSweeper {
    /// Create a new `AnnounceSweeper`.
    pub fn new(target: Arc<dyn AnnounceSweepTarget>) -> Self {
        Self { target }
    }

    /// Run the sweeper loop until `shutdown` signal is received.
    ///
    /// When shutdown arrives, if a sweep is in progress the sweeper
    /// waits up to [`ANNOUNCE_SWEEP_GRACE_PERIOD_SECS`] for it to
    /// finish before forcibly aborting the task.
    pub async fn run(&self, mut shutdown: watch::Receiver<()>) {
        let interval = tokio::time::Duration::from_secs(ANNOUNCE_SWEEP_INTERVAL_SECS);
        let mut next_fire = Instant::now() + interval;
        let mut running_task: Option<tokio::task::JoinHandle<()>> = None;

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    info!("AnnounceSweeper received shutdown signal, exiting");
                    break;
                }
                _ = tokio::time::sleep_until(next_fire), if running_task.is_none() => {
                    let target = Arc::clone(&self.target);
                    let task = tokio::task::spawn(async move {
                        let sweeper = AnnounceSweeper { target };
                        sweeper.run_once().await;
                    });
                    running_task = Some(task);
                    next_fire += interval;
                    // Guard against missed ticks (clock jumped forward).
                    if Instant::now() > next_fire + interval {
                        next_fire = Instant::now() + interval;
                    }
                }
                result = async {
                    match running_task.as_mut() {
                        Some(t) => t.await,
                        None => std::future::pending().await,
                    }
                } => {
                    running_task = None;
                    if let Err(e) = result {
                        error!(%e, "AnnounceSweeper run_once task panicked, continuing");
                    }
                }
            }
        }

        // Grace period: if a sweep is still running, wait then abort
        Self::wait_grace_period(running_task).await;
    }

    /// Wait up to [`ANNOUNCE_SWEEP_GRACE_PERIOD_SECS`] for a running
    /// sweep to finish, then abort it if it does not complete in time.
    async fn wait_grace_period(task: Option<tokio::task::JoinHandle<()>>) {
        let Some(mut task) = task else {
            return;
        };
        let grace = tokio::time::Duration::from_secs(ANNOUNCE_SWEEP_GRACE_PERIOD_SECS);
        info!(
            secs = ANNOUNCE_SWEEP_GRACE_PERIOD_SECS,
            "AnnounceSweeper waiting for running sweep to finish before shutdown"
        );
        tokio::select! {
            result = &mut task => {
                match result {
                    Ok(()) => {
                        info!("AnnounceSweeper sweep completed within grace period");
                    }
                    Err(e) => {
                        error!(%e, "AnnounceSweeper run_once task panicked during graceful shutdown");
                    }
                }
            }
            _ = tokio::time::sleep(grace) => {
                task.abort();
                warn!(
                    secs = ANNOUNCE_SWEEP_GRACE_PERIOD_SECS,
                    "AnnounceSweeper grace period expired, aborting running sweep"
                );
            }
        }
    }

    /// Execute one sweep: check all run-mode children for completed
    /// sessions that haven't had their announce delivered yet.
    pub async fn run_once(&self) {
        let children = self.target.get_run_mode_children().await;

        if children.is_empty() {
            return;
        }

        for (child_id, _parent_id) in &children {
            self.try_sweep_child(child_id).await;
        }
    }

    /// Check a single child session and deliver its announce if it
    /// has completed but the announce hasn't been pushed yet.
    async fn try_sweep_child(&self, child_id: &str) {
        // Verify the child is still in the children table.
        // If it's been removed, the announce was already delivered.
        if self.target.is_child_removed(child_id).await {
            return;
        }

        // Check three-dimensional execution status.
        if !self.target.is_session_idle(child_id).await {
            // Session still active — nothing to do.
            return;
        }

        // Session is idle but still in children table — deliver announce.
        info!(
            child_session_id = %child_id,
            "AnnounceSweeper: child session idle but announce not delivered, pushing"
        );
        self.target
            .try_push_announce(child_id, NotificationPriority::Next)
            .await;
    }
}

impl std::fmt::Debug for AnnounceSweeper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnnounceSweeper").finish()
    }
}
