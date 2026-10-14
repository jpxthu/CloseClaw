//! Scheduler run loops for the DreamingScheduler.
//!
//! Houses the public [`DreamingScheduler::run`] entry point and the two
//! scheduling strategies it dispatches to: the cron-based
//! [`DreamingScheduler::run_cron_loop`] (wall-clock next-fire times) and
//! the fixed-interval fallback
//! [`DreamingScheduler::run_fixed_interval_loop`].

use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::info;

use super::cron::{cron_next_fire, parse_cron_schedule};
use super::{DreamingScheduler, DreamingSchedulerError, LoopEvent, ShutdownCause};

impl DreamingScheduler {
    /// Run the scheduler loop until `shutdown` signal is received.
    ///
    /// Uses cron-based scheduling when a valid expression is available;
    /// falls back to a fixed interval otherwise. Each cycle: dreaming
    /// pipeline first, then mining scan.
    pub async fn run(&mut self, mut shutdown: watch::Receiver<()>) {
        let cron_schedule = self.schedule.as_deref().and_then(parse_cron_schedule);

        if let Some(ref sched) = cron_schedule {
            info!(
                schedule = %sched.display(),
                "DreamingScheduler started with cron schedule"
            );
            self.run_cron_loop(&mut shutdown, cron_next_fire(sched))
                .await;
        } else {
            let interval_secs = self.config.dreaming_interval_secs();
            let interval = tokio::time::Duration::from_secs(interval_secs);
            info!(
                "DreamingScheduler started with fixed interval {}s",
                interval_secs
            );
            self.run_fixed_interval_loop(&mut shutdown, interval).await;
        }
    }

    /// Cron-based scheduling loop: compute next fire time from wall clock.
    async fn run_cron_loop(
        &mut self,
        shutdown: &mut watch::Receiver<()>,
        mut next_fire: tokio::time::Instant,
    ) {
        let mut active_task: Option<JoinHandle<Result<(), DreamingSchedulerError>>> = None;

        loop {
            match self.next_event(shutdown, next_fire).await {
                LoopEvent::Shutdown => {
                    Self::finish_shutdown(&mut active_task, ShutdownCause::Signal).await;
                    break;
                }
                LoopEvent::ConfigClosed => {
                    Self::finish_shutdown(&mut active_task, ShutdownCause::ConfigClosed).await;
                    break;
                }
                LoopEvent::ConfigReload => {
                    self.handle_cron_config_reload(&mut next_fire).await;
                }
                LoopEvent::MiningNotify(session_id) => {
                    self.handle_mining_notify(&session_id).await;
                }
                LoopEvent::Fire => {
                    self.spawn_or_skip_cycle(&mut active_task).await;
                    next_fire = self.next_fire_from_current_schedule();
                }
            }
        }
    }

    /// Recompute the next fire instant from the current `schedule`
    /// (issue #3608): re-parse the expression so a reloaded schedule keeps
    /// governing every subsequent Fire, falling back to the fixed interval
    /// when it is missing or invalid — the same fallback semantics as
    /// [`DreamingScheduler::handle_cron_config_reload`].
    fn next_fire_from_current_schedule(&self) -> tokio::time::Instant {
        match self.schedule.as_deref().and_then(parse_cron_schedule) {
            Some(sched) => cron_next_fire(&sched),
            None => {
                tokio::time::Instant::now()
                    + tokio::time::Duration::from_secs(self.config.dreaming_interval_secs())
            }
        }
    }

    /// Fixed-interval fallback scheduling loop.
    async fn run_fixed_interval_loop(
        &mut self,
        shutdown: &mut watch::Receiver<()>,
        interval: tokio::time::Duration,
    ) {
        let mut next_fire = tokio::time::Instant::now() + interval;
        let mut active_task: Option<JoinHandle<Result<(), DreamingSchedulerError>>> = None;

        loop {
            match self.next_event(shutdown, next_fire).await {
                LoopEvent::Shutdown => {
                    Self::finish_shutdown(&mut active_task, ShutdownCause::Signal).await;
                    break;
                }
                LoopEvent::ConfigClosed => {
                    Self::finish_shutdown(&mut active_task, ShutdownCause::ConfigClosed).await;
                    break;
                }
                LoopEvent::ConfigReload => {
                    self.handle_config_change().await;
                }
                LoopEvent::MiningNotify(session_id) => {
                    self.handle_mining_notify(&session_id).await;
                }
                LoopEvent::Fire => {
                    self.spawn_or_skip_cycle(&mut active_task).await;
                    next_fire = tokio::time::Instant::now() + interval;
                }
            }
        }
    }
}
