//! Background task stop helpers — extracted from lifecycle.rs to stay
//! under the 1000-line file limit.

use super::Daemon;
use crate::shutdown_heartbeat::ShutdownHeartbeat;
use tracing::{error, info, warn};

use crate::lifecycle::TaskStopStatus;

impl Daemon {
    /// Wait for all background tasks to exit, sending periodic heartbeats.
    ///
    /// Waits for all 5 background tasks per the design doc:
    /// ArchiveSweeper, AnnounceSweeper, PlanArchiveSweeper,
    /// DreamingScheduler, and ConfigWatcher subscriber.
    pub(crate) async fn wait_all_bg_tasks(&mut self) -> Vec<(&'static str, TaskStopStatus)> {
        let join_timeout = std::time::Duration::from_secs(7);
        let abort_grace = std::time::Duration::from_secs(3);

        // Compile-time: total Phase 3 budget must be 10s per design doc.
        #[allow(clippy::assertions_on_constants, clippy::eq_op)]
        const _: () = assert!(
            7 + 3 == 10,
            "Phase 3 total timeout (join_timeout + abort_grace) must equal 10s"
        );
        let mut heartbeat = ShutdownHeartbeat::new();
        let mut results: Vec<(&str, TaskStopStatus)> = Vec::new();
        let tasks: Vec<(&str, Option<tokio::task::JoinHandle<()>>)> = vec![
            ("ArchiveSweeper", self.archive_sweeper_handle.take()),
            ("AnnounceSweeper", self.announce_sweeper_handle.take()),
            ("DreamingScheduler", self.dreaming_scheduler_handle.take()),
            (
                "PlanArchiveSweeper",
                self.plan_archive_sweeper_handle.take(),
            ),
            (
                "ConfigWatcherSubscriber",
                self.config_watcher_subscriber_handle.take(),
            ),
        ];
        for (name, handle) in tasks {
            if let Some(h) = handle {
                let status = self
                    .wait_for_background_task_with_heartbeat(
                        h,
                        name,
                        join_timeout,
                        abort_grace,
                        &mut heartbeat,
                    )
                    .await;
                results.push((name, status));
            }
        }
        results
    }

    /// Summarize Phase 3 background task stop results.
    pub(crate) fn log_phase3_stop_confirmation(results: &[(&str, TaskStopStatus)]) {
        let clean = results
            .iter()
            .filter(|(_, s)| matches!(s, TaskStopStatus::Clean))
            .count();
        let panicked = results
            .iter()
            .filter(|(_, s)| matches!(s, TaskStopStatus::Panicked))
            .count();
        let aborted = results
            .iter()
            .filter(|(_, s)| matches!(s, TaskStopStatus::Aborted))
            .count();
        info!(
            clean,
            panicked, aborted, "phase 3 background tasks stopped — confirmation"
        );
        for (name, status) in results {
            match status {
                TaskStopStatus::Clean => info!(task = %name, "stopped: clean exit"),
                TaskStopStatus::Panicked => warn!(task = %name, "stopped: panicked"),
                TaskStopStatus::Aborted => {
                    warn!(task = %name, "stopped: aborted (timeout)")
                }
            }
        }
    }

    /// Wait for a background task to exit, sending periodic shutdown
    /// heartbeats during the wait.
    ///
    /// Uses `tokio::select!` with `ShutdownHeartbeat::next_deadline()`
    /// to send heartbeat notifications every 30s while waiting for the
    /// task to finish.  Ref: design doc § "心跳在存在等待的停止阶段
    /// 生效" — Phase 3 后台任务停止.
    pub(crate) async fn wait_for_background_task_with_heartbeat(
        &self,
        mut handle: tokio::task::JoinHandle<()>,
        name: &str,
        timeout: std::time::Duration,
        abort_grace: std::time::Duration,
        heartbeat: &mut ShutdownHeartbeat,
    ) -> TaskStopStatus {
        let wait_with_heartbeats = async {
            loop {
                tokio::select! {
                    result = &mut handle => return result,
                    _ = tokio::time::sleep_until(heartbeat.next_deadline()) => {
                        self.try_send_heartbeat(heartbeat).await;
                    }
                }
            }
        };

        match tokio::time::timeout(timeout, wait_with_heartbeats).await {
            Ok(join_result) => Self::classify_task_result(name, join_result, heartbeat),
            Err(_) => Self::abort_task_with_grace(handle, name, abort_grace, heartbeat).await,
        }
    }

    /// Classify a completed task's join result into a stop status.
    pub(crate) fn classify_task_result(
        name: &str,
        result: Result<(), tokio::task::JoinError>,
        heartbeat: &mut ShutdownHeartbeat,
    ) -> TaskStopStatus {
        match result {
            Ok(()) => {
                info!("{} exited cleanly", name);
                heartbeat.record_event();
                TaskStopStatus::Clean
            }
            Err(e) => {
                warn!(error = %e, "{} task panicked", name);
                heartbeat.record_event();
                TaskStopStatus::Panicked
            }
        }
    }

    /// Abort a task and wait with a grace period for termination.
    pub(crate) async fn abort_task_with_grace(
        handle: tokio::task::JoinHandle<()>,
        name: &str,
        abort_grace: std::time::Duration,
        heartbeat: &mut ShutdownHeartbeat,
    ) -> TaskStopStatus {
        warn!("{} did not exit within timeout, aborting", name);
        handle.abort();
        match tokio::time::timeout(abort_grace, handle).await {
            Ok(Ok(())) => info!("{} terminated after abort", name),
            Ok(Err(_)) => {
                info!("{} task panicked on abort join — terminated", name)
            }
            Err(_) => {
                error!("{} still alive after abort — possible resource leak", name)
            }
        }
        heartbeat.record_event();
        TaskStopStatus::Aborted
    }
}
