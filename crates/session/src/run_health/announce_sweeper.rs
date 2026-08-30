//! Announce Sweeper — background task for spawn silent-failure protection.
//!
//! Periodically scans for run-mode child sessions that have completed
//! (four-dimensional execution state all zeroed) but whose announce
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
pub(crate) const ANNOUNCE_SWEEP_GRACE_PERIOD_SECS: u64 = 10;

/// Threshold in seconds: a non-idle child session with no new output
/// for longer than this is considered stale (僵死).
const STALE_CHILD_THRESHOLD_SECS: u64 = 300;

/// Returns the stale-child threshold in seconds.
/// Extracted as a function for testability (avoids direct wall-clock
/// dependency in detection logic).
fn stale_child_threshold_secs() -> u64 {
    STALE_CHILD_THRESHOLD_SECS
}

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

    /// Check whether a session's four-dimensional execution
    /// status is `Idle`.
    async fn is_session_idle(&self, session_id: &str) -> bool;

    /// Push an announce event from a completed child to its parent.
    async fn try_push_announce(&self, session_id: &str, priority: NotificationPriority);

    /// Get the timestamp (epoch seconds) of the last output produced
    /// by a session. "Output" means a new assistant message or tool
    /// execution result. Returns `None` if the session is unknown.
    async fn get_last_output_at(&self, session_id: &str) -> Option<i64>;

    /// Check whether a parent session is archived (not in the active
    /// registry or already archived). Used to decide whether to skip
    /// injecting a stale-child notification.
    async fn is_parent_archived(&self, parent_id: &str) -> bool;

    /// Terminate a stale child session and all its descendants.
    ///
    /// Contract:
    /// 1. Kill the child and cascade-terminate all descendants.
    /// 2. If the parent is NOT archived, inject a `Terminated`
    ///    announce event with the stale duration into the parent's
    ///    announce queue.
    /// 3. If the parent IS archived, skip the notification
    ///    (termination still proceeds).
    async fn terminate_stale_child(&self, parent_id: &str, child_id: &str);

    /// Sweep the spawn tree and reclaim residual nodes (GC 兜底).
    ///
    /// Called periodically from the sweeper loop to clean up:
    /// 1. Terminal-status children under active parents
    ///    (滞留「完成待回收」).
    /// 2. Children whose parent session no longer exists
    ///    (ended/archived).
    ///
    /// Default implementation is a no-op; gateway overrides with
    /// [`closeclaw_gateway::spawn_reclaim_gc::sweep_spawn_tree_reclaim`].
    async fn sweep_reclaim(&self) {}
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
    pub(crate) async fn wait_grace_period(task: Option<tokio::task::JoinHandle<()>>) {
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
}

// Sweep logic: stale detection and announce delivery.
impl AnnounceSweeper {
    /// Execute one sweep: check all run-mode children for completed
    /// sessions that haven't had their announce delivered yet, and
    /// detect stale (僵死) children that have been idle too long.
    ///
    /// If `now` is `None`, the current wall-clock time is used.
    /// Accepting an explicit `now` enables deterministic testing
    /// without a real clock dependency.
    pub async fn run_once_with_now(&self, now: Option<i64>) {
        let children = self.target.get_run_mode_children().await;

        if children.is_empty() {
            return;
        }

        let now = now.unwrap_or_else(|| chrono::Utc::now().timestamp());
        for (child_id, parent_id) in &children {
            self.try_sweep_child(child_id).await;
            self.try_detect_stale(parent_id, child_id, now).await;
        }
    }

    /// Execute one sweep using the current wall-clock time.
    pub async fn run_once(&self) {
        self.run_once_with_now(None).await;
        // GC 兜底：每周期回收残留节点（terminal 滞留 + 父 session 已结束）
        self.target.sweep_reclaim().await;
    }

    /// Detect a single non-idle child that may be stale (僵死).
    ///
    /// A child is considered stale when:
    /// - It is NOT idle (still active: Busy or Waiting), AND
    /// - Its last output timestamp is older than the threshold.
    async fn try_detect_stale(&self, parent_id: &str, child_id: &str, now: i64) {
        // Idle children are handled by try_sweep_child; skip them.
        if self.target.is_session_idle(child_id).await {
            return;
        }

        let Some(last_output_at) = self.target.get_last_output_at(child_id).await else {
            // No output recorded yet — not stale.
            return;
        };

        let elapsed = now - last_output_at;
        let threshold = stale_child_threshold_secs() as i64;
        if elapsed <= threshold {
            return;
        }

        warn!(
            child_session_id = %child_id,
            parent_session_id = %parent_id,
            elapsed_secs = elapsed,
            threshold_secs = threshold,
            "AnnounceSweeper: child session stale, terminating"
        );
        self.target.terminate_stale_child(parent_id, child_id).await;
    }

    /// Check a single child session and deliver its announce if it
    /// has completed but the announce hasn't been pushed yet.
    async fn try_sweep_child(&self, child_id: &str) {
        // Verify the child is still in the children table.
        // If it's been removed, the announce was already delivered.
        if self.target.is_child_removed(child_id).await {
            return;
        }

        // Check four-dimensional execution status.
        if !self.target.is_session_idle(child_id).await {
            // Session still active — nothing to do.
            return;
        }

        // Session is idle but still in children table — deliver announce.
        info!(
            child_session_id = %child_id,
            "AnnounceSweeper: child session idle \
                but announce not delivered, pushing"
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
