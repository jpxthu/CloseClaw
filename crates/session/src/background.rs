//! Background task for periodic plan archival.
//!
//! Scans all agent workspaces for completed plan files and archives
//! those exceeding the configured age threshold. Runs as a long-lived
//! background task spawned by the daemon at startup.

use std::path::PathBuf;

use tokio::sync::watch;
use tracing::{debug, error, info, warn};

use crate::plan_archive::{ArchiveConfig, PlanArchiver};

/// Default archival interval in seconds (1 hour).
const DEFAULT_ARCHIVE_INTERVAL_SECS: u64 = 3600;

/// Grace period (seconds) to wait for a running archive sweep to finish
/// before aborting on shutdown.
const ARCHIVE_GRACE_PERIOD_SECS: u64 = 5;

/// Background task that periodically archives completed plan files.
pub struct PlanArchiveTask {
    /// Root config directory containing workspaces/.
    config_dir: PathBuf,
    /// Archive configuration (threshold, etc.).
    archive_config: ArchiveConfig,
}

impl PlanArchiveTask {
    /// Create a new task with the given config directory and threshold.
    pub fn new(config_dir: PathBuf, threshold_days: u64) -> Self {
        Self {
            config_dir,
            archive_config: ArchiveConfig { threshold_days },
        }
    }

    /// Create a new task using default settings (7-day threshold).
    pub fn with_defaults(config_dir: PathBuf) -> Self {
        Self::new(config_dir, crate::plan_archive::DEFAULT_THRESHOLD_DAYS)
    }

    /// Run the archival loop until the shutdown signal is received.
    ///
    /// Executes an initial sweep immediately, then repeats at the configured
    /// interval. Gracefully waits for any in-progress sweep before exiting.
    pub async fn run(&self, mut shutdown: watch::Receiver<()>) {
        let interval = tokio::time::Duration::from_secs(DEFAULT_ARCHIVE_INTERVAL_SECS);
        let mut next_fire = tokio::time::Instant::now() + interval;
        let mut running_task: Option<tokio::task::JoinHandle<()>> = None;

        info!(
            config_dir = %self.config_dir.display(),
            threshold_days = self.archive_config.threshold_days,
            interval_secs = DEFAULT_ARCHIVE_INTERVAL_SECS,
            "PlanArchiveTask started"
        );

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    info!("PlanArchiveTask received shutdown signal, exiting");
                    break;
                }
                _ = tokio::time::sleep_until(next_fire), if running_task.is_none() => {
                    let config_dir = self.config_dir.clone();
                    let threshold_days = self.archive_config.threshold_days;
                    let task = tokio::task::spawn(async move {
                        Self::run_once_static(config_dir, threshold_days).await;
                    });
                    running_task = Some(task);
                    next_fire += interval;
                    // Prevent fire-time drift if the system clock jumped.
                    if tokio::time::Instant::now() > next_fire + interval {
                        next_fire = tokio::time::Instant::now() + interval;
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
                        error!(%e, "PlanArchiveTask sweep panicked, continuing");
                    }
                }
            }
        }

        // Grace period: wait for a running sweep to finish before exiting.
        Self::wait_grace_period(running_task).await;
        info!("PlanArchiveTask stopped");
    }

    /// Execute a single archival sweep across all workspaces.
    pub async fn run_once(&self) {
        Self::run_once_static(self.config_dir.clone(), self.archive_config.threshold_days).await;
    }

    /// Static helper: scan all workspaces and archive eligible plans.
    async fn run_once_static(config_dir: PathBuf, threshold_days: u64) {
        let workspaces_root = config_dir.join("workspaces");
        if !workspaces_root.is_dir() {
            debug!(
                path = %workspaces_root.display(),
                "workspaces directory does not exist, skipping plan archival"
            );
            return;
        }

        let archiver = PlanArchiver::new(threshold_days);
        let mut total_archived = 0u64;

        let agent_entries = match std::fs::read_dir(&workspaces_root) {
            Ok(e) => e,
            Err(e) => {
                warn!(%e, "failed to read workspaces directory");
                return;
            }
        };

        for agent_entry in agent_entries {
            let agent_entry = match agent_entry {
                Ok(e) => e,
                Err(e) => {
                    warn!(%e, "failed to read agent entry");
                    continue;
                }
            };
            let agent_path = agent_entry.path();
            if !agent_path.is_dir() {
                continue;
            }

            let user_entries = match std::fs::read_dir(&agent_path) {
                Ok(e) => e,
                Err(e) => {
                    warn!(agent = %agent_path.display(), %e, "failed to read agent workspace");
                    continue;
                }
            };

            for user_entry in user_entries {
                let user_entry = match user_entry {
                    Ok(e) => e,
                    Err(e) => {
                        warn!(%e, "failed to read user entry");
                        continue;
                    }
                };
                let workspace_dir = user_entry.path();
                if !workspace_dir.is_dir() {
                    continue;
                }

                match archiver.archive(&workspace_dir) {
                    Ok(count) => {
                        if count > 0 {
                            info!(
                                workspace = %workspace_dir.display(),
                                archived = count,
                                "archived completed plans"
                            );
                        }
                        total_archived += count;
                    }
                    Err(e) => {
                        warn!(
                            workspace = %workspace_dir.display(),
                            %e,
                            "failed to archive plans in workspace"
                        );
                    }
                }
            }
        }

        if total_archived > 0 {
            info!(total = total_archived, "plan archival sweep complete");
        } else {
            debug!("plan archival sweep complete — no plans archived");
        }
    }

    /// Wait up to [`ARCHIVE_GRACE_PERIOD_SECS`] for a running sweep to finish.
    async fn wait_grace_period(task: Option<tokio::task::JoinHandle<()>>) {
        let Some(mut task) = task else {
            return;
        };
        let grace = tokio::time::Duration::from_secs(ARCHIVE_GRACE_PERIOD_SECS);
        info!(
            secs = ARCHIVE_GRACE_PERIOD_SECS,
            "PlanArchiveTask waiting for running sweep to finish"
        );
        tokio::select! {
            result = &mut task => {
                match result {
                    Ok(()) => info!("Running archive sweep completed within grace period"),
                    Err(e) => error!(%e, "Archive sweep task panicked during shutdown"),
                }
            }
            _ = tokio::time::sleep(grace) => {
                task.abort();
                warn!(
                    secs = ARCHIVE_GRACE_PERIOD_SECS,
                    "Grace period expired, aborting archive sweep"
                );
            }
        }
    }
}
